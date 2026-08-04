//! Media format conversion pipeline — converts non-native formats to
//! browser-compatible equivalents using FFmpeg.
//!
//! Conversion targets:
//! - Images (HEIC, TIFF, RAW, etc.) → JPEG
//! - Videos (MKV, AVI, MOV, etc.)   → MP4 (H.264/AAC)
//! - Audio  (WMA, AIFF, M4A, etc.)  → MP3
//!
//! Quality is tuned for visual/audible fidelity while keeping file sizes
//! manageable.  FFmpeg must be installed on the host system.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;
use crate::transcode::HwAccelCapability;

// ── GPU acceleration config (set once at startup) ────────────────────────────

/// Cached GPU hardware acceleration capability, set by `init_gpu_config()`.
static GPU_CONFIG: OnceLock<GpuConversionConfig> = OnceLock::new();

struct GpuConversionConfig {
    hwaccel: HwAccelCapability,
    fallback_to_cpu: bool,
}

/// Initialize GPU conversion config. Called once from main.rs at startup.
pub fn init_gpu_config(hwaccel: HwAccelCapability, fallback_to_cpu: bool) {
    let _ = GPU_CONFIG.set(GpuConversionConfig {
        hwaccel,
        fallback_to_cpu,
    });
}

/// Public accessor for the active hardware-acceleration capability.
/// Returns `None` when `init_gpu_config` has not been called yet.
/// Used by the web-preview pipeline so on-the-fly mp4 transcodes
/// honour the same NVENC/QSV/VAAPI path as bulk conversion.
pub fn active_hwaccel() -> Option<&'static HwAccelCapability> {
    GPU_CONFIG.get().map(|c| &c.hwaccel)
}

/// Public accessor for the configured CPU-fallback policy.
pub fn cpu_fallback_enabled() -> bool {
    GPU_CONFIG.get().map(|c| c.fallback_to_cpu).unwrap_or(true)
}

/// Get the current GPU config, or None if not initialized.
fn gpu_config() -> Option<&'static GpuConversionConfig> {
    GPU_CONFIG.get()
}

// ── CPU parallelism planning ─────────────────────────────────────────────────

/// Target libx264 threads per CPU video encode. Software H.264 thread-scaling
/// flattens out past a handful of threads, so on a many-core host we run several
/// encodes in parallel — each capped near this many threads — instead of one
/// thread-hungry encode that leaves cores idle waiting on the encoder's serial
/// dependencies.
const CPU_VIDEO_THREADS_TARGET: usize = 8;

/// Max concurrent GPU video sessions. Hardware encoders (NVENC/QSV/VAAPI) expose
/// only a few simultaneous encode sessions; over-subscribing them just fails or
/// silently serialises, so the video lane is capped low on the GPU path
/// regardless of core count.
const GPU_VIDEO_SESSIONS: usize = 3;

/// Fraction of cores held back for the rest of the server (request handling,
/// encryption, AI/geo). `cores / RESERVE_DIVISOR`, floored at one core, is
/// subtracted from the total so a heavy import never wedges the UI. `8` ⇒ ~12.5%
/// headroom.
const RESERVE_DIVISOR: usize = 8;

/// Concurrency budget for a conversion batch, derived from the host's available
/// CPU parallelism. See [`plan_parallelism`]. Auto-scales from a single-core box
/// (everything serial) to a many-core workstation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversionParallelism {
    /// Concurrent image/audio conversions (each ffmpeg is ~single-threaded, so
    /// this many run at once to fill the usable cores).
    pub fast_lane: usize,
    /// Concurrent video transcodes.
    pub video_lane: usize,
    /// `-threads` budget handed to each CPU (libx264) encode so
    /// `video_lane * video_threads` stays within the usable core budget. Applies
    /// to the CPU path and to the CPU fallback of a failed GPU encode.
    pub video_threads: usize,
}

/// Operator override for the usable-core budget via `SIMPLE_PHOTOS_CONVERSION_JOBS`.
/// `Some(n)` (n ≥ 1) pins the budget directly, bypassing the auto reserve;
/// anything unset / unparseable / non-positive yields `None` (fully automatic).
fn conversion_jobs_override() -> Option<usize> {
    std::env::var("SIMPLE_PHOTOS_CONVERSION_JOBS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
}

/// Turn a usable-core budget into a per-lane plan.
fn plan_from_usable(usable: usize, gpu: bool) -> ConversionParallelism {
    let usable = usable.max(1);
    let fast_lane = usable;
    let video_lane = if gpu {
        // Hardware encoders expose only a handful of sessions.
        usable.min(GPU_VIDEO_SESSIONS)
    } else {
        // Prefer several thread-capped software encodes over one thread-hungry
        // one, since libx264 scaling plateaus quickly.
        usable / CPU_VIDEO_THREADS_TARGET
    }
    .clamp(1, usable);
    // Split the budget across the concurrent encodes so we don't oversubscribe.
    let video_threads = (usable / video_lane).max(1);
    ConversionParallelism {
        fast_lane,
        video_lane,
        video_threads,
    }
}

/// Plan conversion parallelism for a host with `available` CPUs.
///
/// Reserves `available / RESERVE_DIVISOR` cores (at least one) for the rest of
/// the server, then hands the remaining "usable" budget to the fast lane. The
/// video lane is derived from that budget: several thread-capped libx264 encodes
/// on the CPU path, or a small number of hardware sessions on the GPU path.
/// Scales from a single core (everything serial) up to a 1024-thread workstation
/// without hardcoding a ceiling.
pub fn plan_parallelism(available: usize, gpu: bool) -> ConversionParallelism {
    let available = available.max(1);
    let reserve = (available / RESERVE_DIVISOR).max(1);
    let usable = available.saturating_sub(reserve).max(1);
    plan_from_usable(usable, gpu)
}

/// Detect the host's parallelism and plan the conversion lanes, honouring the
/// `SIMPLE_PHOTOS_CONVERSION_JOBS` override when set. `gpu` selects the video
/// lane model (hardware sessions vs. thread-capped software encodes).
pub fn detect_parallelism(gpu: bool) -> ConversionParallelism {
    match conversion_jobs_override() {
        Some(usable) => plan_from_usable(usable, gpu),
        None => plan_parallelism(num_cpus::get(), gpu),
    }
}

// ── Conversion progress tracking ─────────────────────────────────────────────

/// Global conversion progress counters, polled by the frontend banner.
static CONV_ACTIVE: AtomicBool = AtomicBool::new(false);
static CONV_TOTAL: AtomicI64 = AtomicI64::new(0);
static CONV_DONE: AtomicI64 = AtomicI64::new(0);

/// When a client has declared the size of an upcoming batch (see
/// [`batch_start`]), the denominator is *pinned*: per-file (`progress_add`) and
/// per-pass (`progress_start`) auto-registration no longer mutate the total.
/// This is what fixes the "3/4, 5/6, 12/13" jitter (#11) — without a pin, each
/// in-flight file bumps `total` one step ahead of `done`, so the banner never
/// shows the true batch size.
static CONV_PINNED: AtomicBool = AtomicBool::new(false);

/// Epoch-millis when the current conversion batch became active, or `0` when
/// idle. Drives the conversion-banner ETA (item #4) via the shared
/// `status::progress_math` throughput estimator.
static CONV_STARTED_MS: AtomicI64 = AtomicI64::new(0);

/// Epoch-millis of the last *observable* conversion progress — a batch start, a
/// per-file tick, or a per-upload completion. Drives the stuck-job watchdog
/// (#18): while the pipeline reports `active` but this timestamp stops
/// advancing, the pass is wedged (a future hung outside the per-file ffmpeg
/// timeout, a client that declared a batch then disconnected, etc.). `0` when
/// idle so the watchdog has nothing to check.
static CONV_LAST_PROGRESS_MS: AtomicI64 = AtomicI64::new(0);

/// Number of times the watchdog (or a manual admin reset) has force-recovered a
/// stalled conversion pipeline since boot. Surfaced by the status endpoint so a
/// recurring stall is visible instead of silently eating the pipeline.
static CONV_STALL_COUNT: AtomicI64 = AtomicI64::new(0);

/// Epoch-millis of the most recent stall recovery, or `0` if none this boot.
static CONV_LAST_STALL_MS: AtomicI64 = AtomicI64::new(0);

/// Work-weighted ETA ledger for the current conversion batch (#40).
///
/// Deliberately **parallel to** `CONV_TOTAL`/`CONV_DONE` rather than a
/// replacement for them: those are counts, they render the "3 / 4" banner text,
/// and they carry #11's pinned-denominator fix. This tracks bytes per category
/// so the ETA can stop treating a 4K transcode and a HEIC still as equal work.
///
/// A plain `Mutex` — every critical section is a handful of float ops with no
/// `.await`, the same argument `status::registry()` makes.
fn eta_ledger() -> &'static Mutex<crate::progress::ConversionEta> {
    static LEDGER: OnceLock<Mutex<crate::progress::ConversionEta>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(crate::progress::ConversionEta::new()))
}

/// Monotonic-ish seconds for the ETA ledger. Derived from the same wall clock
/// as the rest of this module's timestamps; the ledger tolerates a backwards
/// step by discarding that sample rather than dividing by a negative delta.
fn now_secs() -> f64 {
    now_ms() as f64 / 1000.0
}

/// Register one candidate's work with the ETA ledger, before conversion starts.
/// The pass calls this for every candidate once it has enumerated the batch.
pub fn eta_enqueue(cat: MediaCategory, size_bytes: i64) {
    eta_ledger().lock().unwrap().enqueue(cat, size_bytes);
}

/// Mark that a file of `cat` has begun converting — starts the wall-clock the
/// first throughput sample for that category is measured against.
pub fn eta_start(cat: MediaCategory) {
    eta_ledger().lock().unwrap().start(cat, now_secs());
}

/// Charge one finished file's weight. Called on **success and failure alike**:
/// a failed transcode still consumed the wall-clock the rate is measured
/// against, and skipping it would make the throughput climb silently as
/// failures accumulate — and leave the remaining weight never draining to zero.
pub fn eta_complete(cat: MediaCategory, size_bytes: i64) {
    eta_ledger()
        .lock()
        .unwrap()
        .complete(cat, size_bytes, now_secs());
}

/// Drop the ledger so one batch's weights cannot leak into the next batch's
/// estimate. Paired with the count reset in [`raw_start`] / [`clear_start_clock`].
///
/// Keeps the throughput calibration — see [`crate::progress::ConversionEta::reset`].
fn eta_reset() {
    eta_ledger().lock().unwrap().reset();
}

// ── Throughput calibration, persisted across boots (#40 remainder) ───────────
//
// The seed rates in `crate::progress` are supposed to govern only a machine
// that has never converted anything. Without this, *every* boot is that
// machine: the ledger is process-local and reset at both ends of every batch,
// so a box that has drained a 15k-photo library still quotes the conservative
// compiled-in video rate the next time it starts up.
//
// Stored in `server_settings`, one key per category rather than one JSON blob,
// so a bad value in one category cannot take the other two down with it — and
// so a pass that only converted images can write the image rate without saying
// anything about video.

/// `server_settings` key holding the last measured throughput for `cat`.
fn calibration_key(cat: MediaCategory) -> &'static str {
    match cat {
        MediaCategory::Image => "conversion_rate_image_bytes_per_sec",
        MediaCategory::Audio => "conversion_rate_audio_bytes_per_sec",
        MediaCategory::Video => "conversion_rate_video_bytes_per_sec",
    }
}

/// Read the stored throughputs, keeping only the ones that survive validation.
///
/// Every failure here is **soft**: a missing, unreadable, unparseable or
/// implausible value is dropped from the result and that category stays on its
/// compiled-in seed. That is deliberately the opposite of
/// [`crate::gallery::retention::pruned_through_seq`], which fails *closed* —
/// the cost of ignoring a corrupt retention floor is silent data loss, while the
/// cost of ignoring a corrupt rate is a worse progress bar for one pass.
///
/// Split out from [`load_throughput_calibration`] so it can be tested against a
/// real database without touching the process-wide ledger.
async fn read_calibration(pool: &sqlx::SqlitePool) -> Vec<(MediaCategory, f64)> {
    let mut out = Vec::new();
    for cat in crate::progress::CATEGORIES {
        let key = calibration_key(cat);
        let raw: Option<String> =
            match sqlx::query_scalar("SELECT value FROM server_settings WHERE key = ?")
                .bind(key)
                .fetch_optional(pool)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        error = %e, key,
                        "[CONVERT] Could not read stored conversion throughput; \
                         this category falls back to the compiled-in seed"
                    );
                    continue;
                }
            };
        let Some(raw) = raw else { continue };

        let Ok(parsed) = raw.trim().parse::<f64>() else {
            tracing::warn!(
                key, value = %raw,
                "[CONVERT] Stored conversion throughput is not a number; ignoring it"
            );
            continue;
        };

        // Validated here as well as in `calibrate`, so a bad row is reported
        // with the key that holds it — `calibrate` only sees a float.
        match crate::progress::plausible_rate(parsed) {
            Some(rate) => out.push((cat, rate)),
            None => tracing::warn!(
                key,
                value = parsed,
                "[CONVERT] Stored conversion throughput is not a plausible rate; \
                 ignoring it and using the compiled-in seed"
            ),
        }
    }
    out
}

/// Upsert one row per category. Categories absent from `rates` are left alone —
/// see [`take_measured_rates`] for why that matters.
async fn write_calibration(pool: &sqlx::SqlitePool, rates: &[(MediaCategory, f64)]) {
    for (cat, rate) in rates {
        let key = calibration_key(*cat);
        if let Err(e) = sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(rate.to_string())
        .execute(pool)
        .await
        {
            tracing::warn!(
                error = %e, key,
                "[CONVERT] Could not persist measured conversion throughput; \
                 the next boot falls back to the seed"
            );
        } else {
            tracing::info!(
                key,
                mb_per_sec = rate / (1024.0 * 1024.0),
                "[CONVERT] Stored measured conversion throughput"
            );
        }
    }
}

/// Install stored rates into the ledger as this machine's seeds.
fn apply_calibration(rates: &[(MediaCategory, f64)]) {
    let mut ledger = eta_ledger().lock().unwrap();
    for (cat, rate) in rates {
        if ledger.calibrate(*cat, *rate) {
            tracing::info!(
                key = calibration_key(*cat),
                mb_per_sec = rate / (1024.0 * 1024.0),
                "[CONVERT] Restored measured conversion throughput; the ETA seed \
                 for this category is retired"
            );
        }
    }
}

/// Snapshot what the current batch measured **and** install it as the
/// in-process seed, under one lock so the two can never disagree.
///
/// Updating the in-process copy is not an optimisation: without it the second
/// pass of a boot would be back on the compiled-in seed until a restart re-read
/// the row, because [`eta_reset`] runs at both ends of every batch.
///
/// Categories with no sample are omitted rather than reported as zero — an
/// images-only pass must not overwrite the video rate a mixed pass measured
/// last week.
fn take_measured_rates() -> Vec<(MediaCategory, f64)> {
    let mut ledger = eta_ledger().lock().unwrap();
    let measured: Vec<(MediaCategory, f64)> = crate::progress::CATEGORIES
        .into_iter()
        .filter_map(|cat| ledger.measured_rate(cat).map(|rate| (cat, rate)))
        .collect();
    for (cat, rate) in &measured {
        ledger.calibrate(*cat, *rate);
    }
    measured
}

/// Load previously measured throughputs into the ledger. Called once at boot,
/// before any conversion pass can run.
pub async fn load_throughput_calibration(pool: &sqlx::SqlitePool) {
    let rates = read_calibration(pool).await;
    apply_calibration(&rates);
}

/// Persist what this pass measured. Called once per conversion pass rather than
/// once per file: the value only matters at the *start* of a pass, and a DB
/// write per transcode would put an I/O round trip in the hot loop for nothing.
pub async fn save_throughput_calibration(pool: &sqlx::SqlitePool) {
    let rates = take_measured_rates();
    write_calibration(pool, &rates).await;
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Arm the ETA clock the first time a batch goes active (no-op if already set),
/// so concurrent per-upload passes don't keep resetting the start time.
fn arm_start_clock_if_unset() {
    let _ = CONV_STARTED_MS.compare_exchange(0, now_ms(), Ordering::Relaxed, Ordering::Relaxed);
}

/// Clear the ETA + progress clocks when the batch finishes so stale timestamps
/// can't leak into the next batch's estimate or trip the watchdog while idle.
fn clear_start_clock() {
    CONV_STARTED_MS.store(0, Ordering::Relaxed);
    CONV_LAST_PROGRESS_MS.store(0, Ordering::Relaxed);
    // Drop the weighted ledger with the clocks. NOT redundant with the reset in
    // `raw_start`: the interactive upload path (`progress_add`) re-arms the
    // banner without going through `raw_start`, so an abandoned pass's
    // outstanding weight would otherwise be quoted to the next upload (#40).
    eta_reset();
}

/// Stamp "progress just happened" for the stuck-job watchdog (#18). Called on
/// every event that proves the pipeline is alive: batch start, per-file tick,
/// per-upload completion, and new work arriving.
fn stamp_progress() {
    CONV_LAST_PROGRESS_MS.store(now_ms(), Ordering::Relaxed);
}

/// Unconditionally reset and arm the counters. Internal — callers go through
/// [`progress_start`] (pin-aware) or [`batch_start`] (pin-setting).
fn raw_start(total: i64) {
    CONV_DONE.store(0, Ordering::Relaxed);
    CONV_TOTAL.store(total, Ordering::Relaxed);
    CONV_ACTIVE.store(true, Ordering::Relaxed);
    // A fresh batch resets the weighted ledger too, so the pass that follows
    // enqueues into an empty one (#40).
    eta_reset();
    // Fresh batch → reset the ETA clock and arm the watchdog progress clock.
    let now = now_ms();
    CONV_STARTED_MS.store(now, Ordering::Relaxed);
    CONV_LAST_PROGRESS_MS.store(now, Ordering::Relaxed);
}

/// Start a new conversion batch (resets counters).
///
/// No-op while a client batch is pinned, so a burst of deferred uploads each
/// kicking `run_conversion_pass` can't reset the denominator the client
/// declared mid-flight (#11).
pub fn progress_start(total: i64) {
    if CONV_PINNED.load(Ordering::Relaxed) {
        // Keep the banner visible; ignore the would-be reset.
        CONV_ACTIVE.store(true, Ordering::Relaxed);
        return;
    }
    raw_start(total);
}

/// Pin the denominator to a client-declared `total` for the duration of an
/// upload batch (#11). The web upload loop knows up front how many convertible
/// files it is about to send, so it declares that count once and the banner
/// reads `done/total` throughout instead of tracking one ahead per file.
pub fn batch_start(total: i64) {
    raw_start(total);
    CONV_PINNED.store(true, Ordering::Relaxed);
}

/// Release a client batch pin (paired with [`batch_start`]).
///
/// Called when the client's upload loop finishes. By then every inline
/// conversion for the batch has completed, so we finalize the banner: if the
/// client slightly over-declared (e.g. a file turned out to be a dedup no-op),
/// `done` may be below `total` — clear `active` so the banner doesn't hang.
pub fn batch_end() {
    CONV_PINNED.store(false, Ordering::Relaxed);
    CONV_ACTIVE.store(false, Ordering::Relaxed);
    clear_start_clock();
}

/// Increment the done counter by 1.
pub fn progress_tick() {
    CONV_DONE.fetch_add(1, Ordering::Relaxed);
    stamp_progress();
}

/// Signal that the conversion batch is complete.
pub fn progress_finish() {
    CONV_ACTIVE.store(false, Ordering::Relaxed);
    clear_start_clock();
}

/// Liveness heartbeat for a single long-running transcode.
///
/// A large video makes no `progress_tick` until `convert_file` returns (up to
/// [`GPU_TRANSCODE_TIMEOUT`] + [`CPU_TRANSCODE_TIMEOUT`]), so without a heartbeat
/// the frontend's short-fuse "looks stuck" banner fires while the encode is still
/// healthily running. The ingest loop pulses this every ~20 s *during* a file's
/// conversion. It only advances the watchdog liveness clock — never `done` /
/// `total` — and the caller bounds how long it pulses, so a genuinely wedged file
/// eventually stops heartbeating and the stuck-job watchdog can still recover the
/// pipeline.
pub fn heartbeat() {
    stamp_progress();
}

/// RAII guard around a conversion batch that guarantees the global "active"
/// flag is cleared on **every** exit path — normal completion, early return,
/// a panic inside the pass, or future cancellation.
///
/// This is the watchdog for todo #18's "stuck job blocks subsequent jobs":
/// `run_conversion_pass_inner` sets `CONV_ACTIVE = true` up front and clears it
/// at the very end, with a long convert → register → encrypt loop (dozens of
/// `.await` points) in between. If any of those panics or the spawned task is
/// dropped mid-pass, the old explicit `progress_finish()` was skipped and
/// `CONV_ACTIVE` stayed `true` forever — and because `ingest_pipeline_busy()`
/// keys off that flag, the background AI and geo processors would defer
/// indefinitely, silently wedging the server on large imports. The guard's
/// `Drop` closes that hole regardless of how the pass unwinds.
///
/// On the happy path the caller invokes [`finish`](Self::finish) at the point
/// the batch truly ends (before any follow-on encryption step, to preserve
/// banner timing), which disarms the drop so the flag isn't cleared twice or,
/// worse, cleared out from under a concurrently-pinned client batch.
#[must_use = "hold the guard for the whole pass; dropping it ends the batch"]
pub struct ConversionBatchGuard {
    finished: bool,
}

impl ConversionBatchGuard {
    /// Start a conversion batch of `total` items and arm the drop-guard.
    /// Mirrors [`progress_start`] semantics (a no-op reset while a client batch
    /// is pinned).
    pub fn start(total: i64) -> Self {
        progress_start(total);
        Self { finished: false }
    }

    /// Normal-path completion: end the batch now and disarm the drop-guard so
    /// `Drop` becomes a no-op. Consumes the guard.
    pub fn finish(mut self) {
        self.finished = true;
        progress_finish();
    }
}

impl Drop for ConversionBatchGuard {
    fn drop(&mut self) {
        if !self.finished {
            // Only reached on panic / early return / cancellation — the normal
            // path calls `finish()`. Clear the flag so the pipeline unwedges.
            tracing::warn!(
                "[INGEST] Conversion pass ended without finishing normally \
                 (panic or cancellation) — clearing stuck 'converting' flag so \
                 AI/geo processing can resume"
            );
            progress_finish();
        }
    }
}

/// Register an additional `n` items to the in-flight conversion total
/// **without** resetting the existing `done` counter.  Used by the
/// per-upload conversion path (`photos/upload.rs`) where each upload is
/// its own one-item "batch" but we want the banner to span all
/// concurrent uploads instead of flashing once per file.
///
/// Safe to interleave with `progress_start` (batch ingest) — the
/// running totals just accumulate, and the banner naturally hides once
/// `done == total` and `active` flips back to false via
/// [`progress_finish_one`].
pub fn progress_add(n: i64) {
    // A file entering the pipeline is a liveness signal for the watchdog (#18),
    // whether or not the denominator is pinned.
    stamp_progress();
    // While a client batch is pinned the total is fixed — just keep the banner
    // armed and let `progress_finish_one` advance `done` against the declared
    // denominator (#11).
    if CONV_PINNED.load(Ordering::Relaxed) {
        CONV_ACTIVE.store(true, Ordering::Relaxed);
        return;
    }
    CONV_TOTAL.fetch_add(n, Ordering::Relaxed);
    CONV_ACTIVE.store(true, Ordering::Relaxed);
    // First upload of a per-file batch arms the ETA clock; later ones no-op.
    arm_start_clock_if_unset();
}

/// Counterpart to [`progress_add`] — increments `done` and clears the
/// `active` flag once `done` has caught up to `total`.  This is what
/// keeps the banner visible across many concurrent uploads but lets it
/// hide when the queue drains.
pub fn progress_finish_one() {
    let done = CONV_DONE.fetch_add(1, Ordering::Relaxed) + 1;
    stamp_progress();
    let total = CONV_TOTAL.load(Ordering::Relaxed);
    if done >= total {
        CONV_ACTIVE.store(false, Ordering::Relaxed);
        clear_start_clock();
    }
}

/// Read the current conversion progress snapshot.
/// `done` is clamped to `total` as a safety net against races.
pub fn progress_snapshot() -> (bool, i64, i64) {
    let active = CONV_ACTIVE.load(Ordering::Relaxed);
    let total = CONV_TOTAL.load(Ordering::Relaxed);
    let done = CONV_DONE.load(Ordering::Relaxed).min(total);
    (active, total, done)
}

// ── Stuck-job watchdog (#18) ─────────────────────────────────────────────────

/// Pure stall decision for the watchdog, split out so it can be unit-tested
/// without the process clock or a running pass.
///
/// Stalled ⇔ the pipeline still reports `active` yet no progress has been
/// observed for at least `stall_threshold_ms`. `last_progress_ms == 0` means the
/// pipeline is idle (never started or already finalized) → never stalled, even
/// if `active` momentarily races ahead of the clock.
fn is_stalled(active: bool, last_progress_ms: i64, now: i64, stall_threshold_ms: i64) -> bool {
    active && last_progress_ms > 0 && now.saturating_sub(last_progress_ms) >= stall_threshold_ms
}

/// Watchdog probe: `Some(idle_secs)` when the pipeline is currently stalled per
/// [`is_stalled`] against `stall_threshold_secs`, where `idle_secs` is how long
/// it has been stuck. `None` when healthy or idle. A `0` threshold disables the
/// check (the caller shouldn't even spawn the watchdog in that case, but this is
/// belt-and-suspenders).
pub fn stall_check(stall_threshold_secs: u64) -> Option<u64> {
    if stall_threshold_secs == 0 {
        return None;
    }
    let active = CONV_ACTIVE.load(Ordering::Relaxed);
    let last = CONV_LAST_PROGRESS_MS.load(Ordering::Relaxed);
    let now = now_ms();
    let threshold_ms = (stall_threshold_secs as i64).saturating_mul(1000);
    if is_stalled(active, last, now, threshold_ms) {
        Some((now.saturating_sub(last) / 1000).max(0) as u64)
    } else {
        None
    }
}

/// Force the conversion pipeline back to a clean idle state and record a stall
/// recovery. Used by the watchdog when it detects a wedged pass, and by the
/// admin manual-intervention endpoint.
///
/// Clears the active flag **and** the client batch pin, so a client that
/// declared a batch (`batch_start`) then disconnected can't keep the banner —
/// and the `ingest_pipeline_busy` gate that starves AI/geo — pinned forever.
/// The `CONV_STALL_COUNT` / `CONV_LAST_STALL_MS` telemetry makes the recovery
/// visible in the status endpoint. Idempotent: safe to call when already idle.
pub fn force_reset(reason: &str) {
    let (active, total, done) = progress_snapshot();
    CONV_STALL_COUNT.fetch_add(1, Ordering::Relaxed);
    CONV_LAST_STALL_MS.store(now_ms(), Ordering::Relaxed);
    CONV_PINNED.store(false, Ordering::Relaxed);
    CONV_ACTIVE.store(false, Ordering::Relaxed);
    clear_start_clock();
    tracing::error!(
        reason = %reason,
        was_active = active,
        done,
        total,
        "[INGEST] Conversion pipeline force-reset (stuck-job recovery, #18) — \
         cleared 'converting' flag and any client batch pin so AI/geo processing \
         and the banner can resume"
    );
}

/// Read the stall telemetry: `(last_progress_ms, stall_count, last_stall_ms)`.
pub fn stall_telemetry() -> (i64, i64, i64) {
    (
        CONV_LAST_PROGRESS_MS.load(Ordering::Relaxed),
        CONV_STALL_COUNT.load(Ordering::Relaxed),
        CONV_LAST_STALL_MS.load(Ordering::Relaxed),
    )
}

// ── Media categories ─────────────────────────────────────────────────────────

/// Broad media category used to select conversion parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCategory {
    Image,
    Video,
    Audio,
}

/// Describes the target format for a conversion.
#[derive(Debug, Clone)]
pub struct ConversionTarget {
    pub extension: &'static str,
    pub mime_type: &'static str,
    pub category: MediaCategory,
}

// ── Extension → target mapping ───────────────────────────────────────────────

/// Determine the conversion target for a file based on its extension.
/// Returns `None` if the file is already a native format or is not a
/// recognised convertible format.
pub fn conversion_target(filename: &str) -> Option<ConversionTarget> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        // ── Images → JPEG ────────────────────────────────────────────
        "heic" | "heif"                                     // Apple
        | "tiff" | "tif"                                    // Tagged Image
        | "hdr"                                             // Radiance HDR
        | "exr"                                             // OpenEXR
        | "psd"                                             // Photoshop
        | "tga"                                             // Targa
        | "pcx"                                             // PC Paintbrush
        | "ppm" | "pgm" | "pbm" | "pnm"                    // Netpbm
        | "xbm" | "xpm"                                    // X11 bitmap
        | "jp2" | "j2k" | "jpx"                            // JPEG 2000
        | "jxl"                                             // JPEG XL
        | "jfif" | "jpe"                                    // JPEG variants
        | "cur"                                             // Windows cursor
        => Some(ConversionTarget {
            extension: "jpg",
            mime_type: "image/jpeg",
            category: MediaCategory::Image,
        }),

        // ── Videos → MP4 (H.264 + AAC) ──────────────────────────────
        "mkv"                                               // Matroska
        | "avi"                                             // AVI
        | "wmv"                                             // Windows Media
        | "mov"                                             // QuickTime
        | "m4v"                                             // iTunes Video
        | "flv" | "f4v"                                     // Flash Video
        | "3gp" | "3g2"                                     // 3GPP
        | "mpg" | "mpeg"                                    // MPEG-1/2
        | "ts" | "mts" | "m2ts"                             // MPEG transport
        | "vob"                                             // DVD
        | "asf"                                             // ASF container
        | "rm" | "rmvb"                                     // RealMedia
        | "divx"                                            // DivX
        | "ogv"                                             // Ogg Video
        | "mxf"                                             // Material Exchange
        | "dv"                                              // Digital Video
        | "hevc" | "h264" | "h265"                          // Raw codec streams
        => Some(ConversionTarget {
            extension: "mp4",
            mime_type: "video/mp4",
            category: MediaCategory::Video,
        }),

        // ── Audio → MP3 ─────────────────────────────────────────────
        "wma"                                               // Windows Media Audio
        | "aiff" | "aif"                                    // Apple AIFF
        | "m4a"                                             // AAC container
        | "aac"                                             // Raw AAC
        | "wv"                                              // WavPack
        | "ape"                                             // Monkey's Audio
        | "opus"                                            // Opus
        | "ra" | "ram"                                      // RealAudio
        | "amr"                                             // Adaptive Multi-Rate
        | "ac3"                                             // Dolby AC3
        | "dts"                                             // DTS audio
        | "tta"                                             // True Audio
        | "mka"                                             // Matroska audio
        | "au" | "snd"                                      // Sun/NeXT audio
        | "caf"                                             // Core Audio
        | "spx"                                             // Speex
        | "dsf" | "dff"                                     // DSD audio
        => Some(ConversionTarget {
            extension: "mp3",
            mime_type: "audio/mpeg",
            category: MediaCategory::Audio,
        }),

        _ => None,
    }
}

/// Check whether a file can be converted to a browser-native format.
pub fn is_convertible(filename: &str) -> bool {
    conversion_target(filename).is_some()
}

/// Media-type string for the database (`photo`, `video`, `audio`, `gif`).
pub fn media_type_str(cat: MediaCategory) -> &'static str {
    match cat {
        MediaCategory::Image => "photo",
        MediaCategory::Video => "video",
        MediaCategory::Audio => "audio",
    }
}

/// Conversion ordering priority (lower runs first).
///
/// Image and audio conversions finish in well under a second; a single video
/// transcode can take minutes (GPU attempt + CPU fallback, each capped at the
/// 600 s ffmpeg timeout). The ingest pass is sequential, so enumerating videos
/// first makes a mixed import look frozen on the first big file (#10). Ordering
/// fast formats ahead of videos keeps progress visibly moving and pushes the
/// slow transcodes to the end of the batch.
pub fn conversion_priority(cat: MediaCategory) -> u8 {
    match cat {
        MediaCategory::Image => 0,
        MediaCategory::Audio => 1,
        MediaCategory::Video => 2,
    }
}

// ── Per-attempt transcode timeouts ───────────────────────────────────────────

/// Timeout for the GPU (hardware) video encode attempt.
///
/// Deliberately short. A hardware encode that hasn't finished in this window is
/// almost always *hung* (unsupported codec path, driver/VAAPI stall) rather than
/// merely slow — a working NVENC/QSV/VAAPI transcode of a normal clip completes
/// in seconds. Failing over to the CPU encoder quickly caps the worst-case
/// per-file time: without this, a pathological video burned the full 600 s GPU
/// budget AND then the full 600 s CPU budget (~20 min of zero visible progress),
/// which is what made the conversion banner read "stuck" (#18 follow-up).
const GPU_TRANSCODE_TIMEOUT: Duration = Duration::from_secs(120);

/// Timeout for the CPU (libx264) video encode. A software encode of a long clip
/// can legitimately take minutes, so this keeps the longer budget.
const CPU_TRANSCODE_TIMEOUT: Duration = Duration::from_secs(600);

// ── FFmpeg conversion ────────────────────────────────────────────────────────

/// Convert a media file to its browser-native target format.
///
/// Uses quality-tuned FFmpeg parameters:
/// - **Images** → JPEG at `-q:v 2` (near-lossless, ~92% quality)
/// - **Videos** → MP4 H.264 at `-crf 20 -preset medium`, AAC 192 kbps
/// - **Audio**  → MP3 at 192 kbps via libmp3lame
///
/// Image conversion is FFmpeg-only (no ImageMagick). HEIC/HEIF decodes natively
/// via FFmpeg's mov demuxer + HEVC decoder; RAW camera formats are unsupported.
///
/// For video conversions, uses GPU-accelerated encoding when available
/// (configured via `init_gpu_config()` at startup).
/// `video_threads` caps the `-threads` handed to the CPU (libx264) encoder so a
/// batch running several encodes in parallel doesn't oversubscribe the cores.
/// `None` lets ffmpeg auto-detect (all cores) — appropriate for a lone, serial
/// transcode such as a single interactive upload.
pub async fn convert_file(
    input: &Path,
    output: &Path,
    target: &ConversionTarget,
    video_threads: Option<usize>,
) -> Result<(), String> {
    // ── Path-injection sanitizer ─────────────────────────────────────────
    // Canonicalize input (must already exist) and the output's parent so all
    // subsequent filesystem operations work against fully resolved paths.
    // We then verify the supplied paths cannot escape those canonical
    // ancestors. This is the standard barrier for the rust/path-injection
    // CodeQL query and is defense-in-depth on top of caller-side validation.
    let canonical_input = tokio::fs::canonicalize(input)
        .await
        .map_err(|e| format!("Canonicalize input path: {e}"))?;

    let output_parent = output
        .parent()
        .ok_or("Output path has no parent directory")?;
    tokio::fs::create_dir_all(output_parent)
        .await
        .map_err(|e| format!("Create output directory: {e}"))?;
    let canonical_output_parent = tokio::fs::canonicalize(output_parent)
        .await
        .map_err(|e| format!("Canonicalize output directory: {e}"))?;
    let output_file_name = output
        .file_name()
        .ok_or("Output path has no file name component")?;
    let canonical_output = canonical_output_parent.join(output_file_name);
    if !canonical_output.starts_with(&canonical_output_parent) {
        return Err("Output path escapes its parent directory".into());
    }

    let input_str = canonical_input
        .to_str()
        .ok_or("Invalid input path encoding")?;
    let output_str = canonical_output
        .to_str()
        .ok_or("Invalid output path encoding")?;

    let success = match target.category {
        MediaCategory::Image => convert_image(input_str, output_str).await,
        MediaCategory::Video => {
            let gpu = gpu_config();
            convert_video(
                input_str,
                output_str,
                gpu.map(|g| &g.hwaccel),
                gpu.map(|g| g.fallback_to_cpu).unwrap_or(true),
                video_threads,
                // First-pass conversion always targets source resolution. Ladder
                // rungs are a separate, lower-priority pass — a user must never
                // wait on a secondary rendition to see their video at all.
                None,
            )
            .await
        }
        MediaCategory::Audio => convert_audio(input_str, output_str).await,
    };

    if !success {
        let _ = tokio::fs::remove_file(&canonical_output).await;
        return Err(format!(
            "Conversion failed for '{}' → .{}",
            canonical_input
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?"),
            target.extension,
        ));
    }

    // Verify the output file exists and is non-empty.
    match tokio::fs::metadata(&canonical_output).await {
        Ok(m) if m.len() > 0 => Ok(()),
        Ok(_) => {
            let _ = tokio::fs::remove_file(&canonical_output).await;
            Err("Conversion produced an empty file".into())
        }
        Err(e) => Err(format!("Output file missing after conversion: {e}")),
    }
}

// ── Format-specific converters ───────────────────────────────────────────────

/// Image → JPEG via FFmpeg.
///
/// FFmpeg is the *only* image converter we depend on — no ImageMagick — to keep
/// the install to a single media tool. HEIC/HEIF (Apple's default camera format)
/// decodes natively here: FFmpeg reads the ISOBMFF/HEIF container via its `mov`
/// demuxer and decodes the still image with the built-in HEVC decoder, so no
/// libheif build flag is needed. (RAW camera formats are intentionally
/// unsupported — they'd require per-vendor decoders we don't ship.)
async fn convert_image(input: &str, output: &str) -> bool {
    tracing::debug!(input = %input, output = %output, "Image conversion: starting JPEG conversion");
    // FFmpeg: high-quality JPEG output (-q:v 2 ≈ 92% quality).
    let mut cmd = crate::process::background_command("ffmpeg");
    cmd.args(["-y", "-i", input, "-q:v", "2", output])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let ffmpeg =
        crate::process::status_with_timeout(&mut cmd, std::time::Duration::from_secs(600)).await;

    let ok = matches!(ffmpeg, Ok(s) if s.success());
    if ok {
        tracing::debug!(input = %input, "Image conversion: FFmpeg JPEG conversion succeeded");
    } else {
        tracing::warn!(input = %input, "Image conversion failed");
    }
    ok
}

/// Transcode a video to an explicit ladder rung (#49).
///
/// The ladder's entry point into this module. It exists so the generation pass
/// does not have to know about `GPU_CONFIG`, CPU fallback or thread budgeting —
/// all three are decisions this file already owns for first-pass conversion,
/// and a second copy of them would drift.
///
/// `rung` is a concrete `(width, height)` from `ladder::rung_dimensions`, which
/// derived it from a *probe of this file*. Passing dimensions taken from
/// `photos.width`/`height` instead squashes the frame: those columns are
/// transposed for part of the live library (see `transcode::rung_queue`).
pub(crate) async fn transcode_to_rung(
    input: &std::path::Path,
    output: &std::path::Path,
    rung: (i64, i64),
) -> Result<(), String> {
    let input_str = input.to_str().ok_or("Invalid input path encoding")?;
    let output_str = output.to_str().ok_or("Invalid output path encoding")?;

    let gpu = gpu_config();
    // Share the batch thread budget rather than letting a background rendition
    // take every core: a ladder encode runs alongside first-pass conversions and
    // must never be the reason a user waits to see their video at all.
    let threads = plan_parallelism(num_cpus::get(), gpu.map(|g| g.hwaccel.is_gpu()).unwrap_or(false))
        .video_threads;

    let ok = convert_video(
        input_str,
        output_str,
        gpu.map(|g| &g.hwaccel),
        gpu.map(|g| g.fallback_to_cpu).unwrap_or(true),
        Some(threads),
        Some(rung),
    )
    .await;

    if !ok {
        // The partial output is removed here rather than by the caller: an
        // interrupted encode leaves a truncated MP4 that is indistinguishable
        // from a complete one by size alone, and encrypting it would publish a
        // rendition that plays for two seconds and stops.
        let _ = tokio::fs::remove_file(output).await;
        return Err(format!("ffmpeg failed to produce the {}x{} rung", rung.0, rung.1));
    }

    match tokio::fs::metadata(output).await {
        Ok(m) if m.len() > 0 => Ok(()),
        Ok(_) => {
            let _ = tokio::fs::remove_file(output).await;
            Err("ffmpeg reported success but produced an empty file".into())
        }
        Err(e) => Err(format!("rung output missing after a successful encode: {e}")),
    }
}

/// Video → MP4 (H.264 + AAC).  Quality-tuned for clarity at reasonable sizes.
/// When a GPU `hwaccel` capability is provided, uses hardware-accelerated
/// encoding.  Falls back to CPU (libx264) if the GPU transcode fails and
/// `fallback_to_cpu` is true.
pub(crate) async fn convert_video(
    input: &str,
    output: &str,
    hwaccel: Option<&HwAccelCapability>,
    fallback_to_cpu: bool,
    video_threads: Option<usize>,
    rung: crate::transcode::ffmpeg_gpu::RungSize,
) -> bool {
    // Diagnostics: log exactly which path was selected for every video.
    // Without this, operators report "still using CPU!" and we have no
    // way to tell whether the GPU branch was even considered.
    match hwaccel {
        Some(hw) if hw.is_gpu() => tracing::debug!(
            input = %input,
            encoder = %hw.video_encoder,
            "convert_video: GPU path selected"
        ),
        Some(_) => tracing::warn!(
            input = %input,
            "convert_video: hwaccel registered as CPU (probe found no GPU encoder)"
        ),
        None => tracing::warn!(
            input = %input,
            "convert_video: no hwaccel config registered — init_gpu_config not called?"
        ),
    }

    // Try GPU path first if available
    if let Some(hw) = hwaccel {
        if hw.is_gpu() {
            let args =
                crate::transcode::ffmpeg_gpu::build_video_transcode_args(input, output, hw, rung);
            tracing::info!(
                encoder = %hw.video_encoder,
                accel = %hw.accel_type,
                device = ?hw.device,
                input = %input,
                output = %output,
                "GPU transcode: starting hardware-accelerated video conversion"
            );
            tracing::debug!(
                ffmpeg_args = ?args,
                "GPU transcode: FFmpeg command arguments"
            );
            let gpu_start = std::time::Instant::now();
            let mut cmd = crate::process::background_command("ffmpeg");
            cmd.args(&args).stdout(std::process::Stdio::null());
            let result = crate::process::run_with_timeout(&mut cmd, GPU_TRANSCODE_TIMEOUT).await;

            let gpu_ok = matches!(&result, Ok(out) if out.status.success());

            if gpu_ok {
                tracing::info!(
                    encoder = %hw.video_encoder,
                    elapsed_ms = gpu_start.elapsed().as_millis(),
                    input = %input,
                    "GPU transcode: hardware-accelerated conversion succeeded"
                );
                return true;
            }

            // Log the actual FFmpeg error so operators can diagnose failures.
            let ffmpeg_stderr = match &result {
                Ok(out) => String::from_utf8_lossy(&out.stderr).to_string(),
                Err(e) => e.clone(),
            };
            let last_lines: String = ffmpeg_stderr
                .lines()
                .rev()
                .take(10)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");

            if !fallback_to_cpu {
                tracing::error!(
                    encoder = %hw.video_encoder,
                    elapsed_ms = gpu_start.elapsed().as_millis(),
                    ffmpeg_error = %last_lines,
                    "GPU transcode: hardware conversion failed and CPU fallback is disabled"
                );
                return false;
            }

            tracing::warn!(
                encoder = %hw.video_encoder,
                elapsed_ms = gpu_start.elapsed().as_millis(),
                ffmpeg_error = %last_lines,
                "GPU transcode: hardware conversion failed — retrying with CPU libx264"
            );
            // Remove partial output before retry
            let _ = tokio::fs::remove_file(output).await; // codeql[rust/path-injection] -- path is server temp dir + UUID; ext restricted to alphanumeric at call sites
        }
    }

    // CPU fallback (original path)
    tracing::info!(
        input = %input,
        output = %output,
        encoder = "libx264",
        "GPU transcode: running CPU software encoding"
    );
    let cpu_start = std::time::Instant::now();
    let mut cmd = crate::process::background_command("ffmpeg");
    // Bound the encoder's thread count when the caller runs several encodes in
    // parallel (bulk import), so `video_lane * video_threads` cores aren't
    // oversubscribed. `0` = ffmpeg auto (all cores) for a lone/interactive encode.
    // MUST honour the same rung the GPU path above was given. These args used
    // to be an inline copy holding their own hardcoded scale filter; see
    // `build_cpu_fallback_args` for why that becomes a silent correctness bug
    // once a rung can be requested.
    cmd.args(crate::transcode::ffmpeg_gpu::build_cpu_fallback_args(
        input,
        output,
        video_threads,
        rung,
    ))
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    let status = crate::process::status_with_timeout(&mut cmd, CPU_TRANSCODE_TIMEOUT).await;
    let ok = matches!(status, Ok(s) if s.success());
    if ok {
        tracing::info!(
            input = %input,
            elapsed_ms = cpu_start.elapsed().as_millis(),
            "GPU transcode: CPU software encoding succeeded"
        );
    } else {
        tracing::error!(
            input = %input,
            elapsed_ms = cpu_start.elapsed().as_millis(),
            "GPU transcode: CPU software encoding failed"
        );
    }
    ok
}

/// Audio → MP3 (LAME).
async fn convert_audio(input: &str, output: &str) -> bool {
    tracing::debug!(input = %input, output = %output, "Audio conversion: starting MP3 conversion");
    tracing::debug!(input = %input, output = %output, "Audio conversion: starting MP3 conversion");
    let mut cmd = crate::process::background_command("ffmpeg");
    cmd.args([
        "-y",
        "-i",
        input,
        "-codec:a",
        "libmp3lame",
        "-b:a",
        "192k",
        output,
    ])
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    let status =
        crate::process::status_with_timeout(&mut cmd, std::time::Duration::from_secs(600)).await;

    matches!(status, Ok(s) if s.success())
}

// ── Conversion status endpoint ───────────────────────────────────────────────

/// Estimated seconds remaining for the active conversion batch (item #4, #40).
///
/// Prefers the **work-weighted, per-category** estimator ([`crate::progress::ConversionEta`]),
/// which is what makes a mixed image-then-video queue's ETA survive the category
/// boundary. Falls back to the count-based [`crate::progress::progress_math`]
/// when the ledger is empty — that is the client-declared upload batch
/// (`POST /api/admin/conversion-batch/start`), whose wire shape carries a count
/// and no sizes, so there is no weight to estimate from. `None` when idle, or
/// when neither estimator has anything to go on.
pub fn conversion_eta_seconds() -> Option<f64> {
    let (active, total, done) = progress_snapshot();
    if !active {
        return None;
    }

    // The conversion pass populates this; a client-declared upload batch does not.
    if let Some(eta) = eta_ledger().lock().unwrap().eta_seconds() {
        return Some(eta);
    }

    let started = CONV_STARTED_MS.load(Ordering::Relaxed);
    if started == 0 {
        return None;
    }
    let elapsed = (now_ms() - started) as f64 / 1000.0;
    let remaining = (total - done).max(0);
    let (_done, eta) = crate::progress::progress_math(total, remaining, elapsed);
    eta
}

#[derive(Debug, Serialize)]
pub struct ConversionStatusResponse {
    pub active: bool,
    pub total: i64,
    pub done: i64,
    /// Estimated seconds remaining, or `null` until throughput is known (item #4).
    pub eta_seconds: Option<f64>,
    /// Epoch-ms of the last observed progress (batch start / tick / completion),
    /// or `0` when idle. Lets the admin surface a frozen pipeline (#18).
    pub last_progress_at: i64,
    /// How many times the watchdog (or a manual reset) has recovered a stalled
    /// pipeline since boot. A climbing value flags a recurring wedge.
    pub stall_count: i64,
    /// Epoch-ms of the most recent stall recovery, or `0` if none this boot.
    pub last_stall_at: i64,
}

/// Build the status response from the live counters + ETA + watchdog telemetry.
fn conversion_status_response() -> ConversionStatusResponse {
    let (active, total, done) = progress_snapshot();
    let (last_progress_at, stall_count, last_stall_at) = stall_telemetry();
    ConversionStatusResponse {
        active,
        total,
        done,
        eta_seconds: conversion_eta_seconds(),
        last_progress_at,
        stall_count,
        last_stall_at,
    }
}

/// GET /api/admin/conversion-status
pub async fn conversion_status(
    _auth: AuthUser,
) -> Result<Json<ConversionStatusResponse>, AppError> {
    Ok(Json(conversion_status_response()))
}

#[derive(Debug, serde::Deserialize)]
pub struct ConversionBatchStartRequest {
    /// Number of convertible files the client is about to upload in this batch.
    pub total: i64,
}

/// POST /api/admin/conversion-batch/start
///
/// Pins the conversion-banner denominator to the client-declared batch size so
/// it reads `n/total` throughout a multi-file upload instead of tracking one
/// ahead (#11). Paired with `conversion-batch/end`.
pub async fn conversion_batch_start(
    _auth: AuthUser,
    Json(req): Json<ConversionBatchStartRequest>,
) -> Result<Json<ConversionStatusResponse>, AppError> {
    if req.total <= 0 {
        return Err(AppError::BadRequest(
            "batch total must be positive".to_string(),
        ));
    }
    batch_start(req.total);
    Ok(Json(conversion_status_response()))
}

/// POST /api/admin/conversion-batch/end
///
/// Releases the pin set by `conversion-batch/start` and finalizes the banner.
pub async fn conversion_batch_end(
    _auth: AuthUser,
) -> Result<Json<ConversionStatusResponse>, AppError> {
    batch_end();
    Ok(Json(conversion_status_response()))
}

/// POST /api/admin/conversion/reset
///
/// Manual intervention for a stuck conversion pipeline (#18). Force-clears the
/// "converting" flag and any client batch pin so the background AI/geo
/// processors — gated by `ingest_pipeline_busy` — and the conversion banner
/// resume immediately. This is the operator escape hatch that pairs with the
/// automatic watchdog; the returned snapshot lets the admin UI confirm the
/// pipeline is back to idle. Idempotent and safe to call when already idle.
pub async fn conversion_reset(_auth: AuthUser) -> Result<Json<ConversionStatusResponse>, AppError> {
    force_reset("manual admin reset via /admin/conversion/reset");
    Ok(Json(conversion_status_response()))
}

#[derive(Debug, serde::Serialize)]
pub struct RetryFailedConversionsResponse {
    /// How many retired files were re-admitted to the conversion queue.
    pub cleared: u64,
}

/// POST /api/admin/conversion/retry-failed
///
/// Clears every `conversion_failed` skip row for the caller, so files retired by
/// #40's three-strike cap are attempted again on the next pass.
///
/// The cap's automatic escape hatch is a change to the file on disk (migration
/// 031's size/mtime invalidation), which covers "the file was broken and I
/// replaced it". It does **not** cover the case this endpoint exists for: the
/// file is unchanged and the *server* got better — a new ffmpeg, a GPU driver
/// fix, hardware acceleration enabled. Telling an admin to touch several hundred
/// files to pick that up is not an escape hatch.
///
/// Scoped to `conversion_failed` on purpose. `hash_duplicate` and
/// `gallery_hidden` are deterministic verdicts that re-clearing would only make
/// the scan re-derive at real cost — and in the duplicate case, re-hash the
/// whole Takeout library, which is the disk thrash migration 031 removed.
///
/// Idempotent: clearing when nothing is retired reports `cleared: 0`.
pub async fn conversion_retry_failed(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<RetryFailedConversionsResponse>, AppError> {
    crate::setup::admin::require_admin(&state, &auth).await?;

    let cleared = sqlx::query("DELETE FROM scan_skipped_paths WHERE user_id = ? AND reason = ?")
        .bind(&auth.user_id)
        .bind(crate::photos::scan_skip::REASON_CONVERSION_FAILED)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!(
                error = %e,
                "[INGEST] Failed to clear retired conversions for retry"
            );
            AppError::Internal("failed to clear retired conversions".to_string())
        })?
        .rows_affected();

    tracing::info!(
        cleared,
        "[INGEST] Admin re-admitted retired files to the conversion queue"
    );

    Ok(Json(RetryFailedConversionsResponse { cleared }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes tests that mutate the global conversion-progress atomics so
    /// cargo's parallel test threads don't race on shared state. Poison-tolerant
    /// so a panic in one test (e.g. the guard-on-panic test) can't wedge others.
    fn global_state_lock() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        L.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    // ── ETA ledger wiring (#40) ─────────────────────────────────────────────
    //
    // The arithmetic is pinned by pure tests in `crate::progress`. These cover
    // the part hand-wired here, which those cannot reach: that the weighted
    // ledger is *preferred* over the count-based estimator, that it is reset by
    // the batch lifecycle, and that an empty ledger falls back rather than
    // suppressing the ETA entirely.

    #[test]
    fn a_weighted_ledger_outranks_the_count_based_estimator() {
        let _lock = global_state_lock();
        batch_end();

        // 10 items by count; by weight, one 2 GB video dominates.
        let _guard = ConversionBatchGuard::start(10);
        eta_enqueue(MediaCategory::Image, 10 * 1024 * 1024);
        eta_enqueue(MediaCategory::Video, 2 * 1024 * 1024 * 1024);
        eta_start(MediaCategory::Image);
        eta_complete(MediaCategory::Image, 10 * 1024 * 1024);

        let eta = conversion_eta_seconds().expect("ledger has outstanding work");
        // 2 GB of video at the seeded rate is ~1000s. The count-based estimator
        // would report a fraction of a second here (1 of 10 done, instantly).
        assert!(
            eta > 300.0,
            "the weighted ledger must win; got {eta:.1}s, which is the \
             count-based answer"
        );
        batch_end();
    }

    #[test]
    fn an_empty_ledger_falls_back_to_the_count_based_estimator() {
        let _lock = global_state_lock();
        batch_end();

        // The client-declared upload path: a count, no sizes. Nothing is
        // enqueued, so the ETA must still come from somewhere.
        batch_start(4);
        std::thread::sleep(std::time::Duration::from_millis(15));
        progress_tick();

        assert!(
            conversion_eta_seconds().is_some(),
            "an upload batch carries no weight, so the count-based estimator \
             must still answer — otherwise #40 removes the ETA it was fixing"
        );
        batch_end();
    }

    #[test]
    fn a_new_batch_does_not_inherit_the_previous_batch_s_weights() {
        let _lock = global_state_lock();
        batch_end();

        {
            let _guard = ConversionBatchGuard::start(1);
            eta_enqueue(MediaCategory::Video, 4 * 1024 * 1024 * 1024);
        }
        // Guard dropped ⇒ batch finished ⇒ ledger cleared. A fresh pass that
        // enqueues one small image must not be quoted the old 4 GB tail.
        let _guard = ConversionBatchGuard::start(1);
        eta_enqueue(MediaCategory::Image, 1024 * 1024);
        let eta = conversion_eta_seconds().expect("one image outstanding");
        assert!(
            eta < 10.0,
            "a finished batch's weights leaked into the next estimate: {eta:.1}s"
        );
        batch_end();
    }

    /// The reset in `clear_start_clock` is NOT redundant with the one in
    /// `raw_start`, and this is the path that proves it.
    ///
    /// `progress_add` — the interactive upload path — arms the banner *without*
    /// going through `raw_start`, so it performs no reset of its own. If a
    /// conversion pass is abandoned mid-flight (the guard's `Drop`, #18) with
    /// weight still outstanding, and only `raw_start` reset the ledger, the very
    /// next upload would be quoted the dead pass's video tail.
    #[test]
    fn an_abandoned_pass_does_not_haunt_the_next_upload_s_eta() {
        let _lock = global_state_lock();
        batch_end();

        {
            let _guard = ConversionBatchGuard::start(2);
            // A 4 GB tail that is never charged — the pass is cancelled below.
            eta_enqueue(MediaCategory::Video, 4 * 1024 * 1024 * 1024);
        } // Drop ⇒ progress_finish ⇒ clear_start_clock.

        // An interactive upload re-arms the banner. No new batch, no `raw_start`.
        progress_add(1);
        let eta = conversion_eta_seconds();
        assert!(
            eta.is_none_or(|e| e < 100.0),
            "the abandoned pass's 4 GB tail leaked into an unrelated upload's \
             ETA: {eta:?}"
        );
        batch_end();
    }

    // ── Cross-pass throughput calibration (#40 remainder) ───────────────────

    /// Wipe the process-wide ledger *including* its calibration. `eta_reset`
    /// deliberately keeps the calibration, which is exactly what these tests
    /// are about, so they need a way back to a virgin machine.
    fn reset_ledger_completely() {
        *eta_ledger().lock().unwrap() = crate::progress::ConversionEta::new();
    }

    /// The whole point of this half of #40. `raw_start` resets the ledger at the
    /// top of every pass; if that also dropped the calibration, a box would be
    /// back on the conservative compiled-in video seed for its second pass —
    /// and for every pass after a restart.
    ///
    /// Verified RED by restoring `ConversionEta::reset` to `*self =
    /// Self::new()`: the estimate returns to the seed and reads ~500s.
    #[test]
    fn a_new_batch_keeps_the_measured_throughput() {
        let _lock = global_state_lock();
        batch_end();
        reset_ledger_completely();

        // Stand in for a pass that measured ~20 MB/s on video. The arithmetic
        // that produces this figure is pinned in `crate::progress`; what is
        // under test here is that the batch lifecycle does not throw it away.
        assert!(eta_ledger()
            .lock()
            .unwrap()
            .calibrate(MediaCategory::Video, 20.0 * 1024.0 * 1024.0));

        let _guard = ConversionBatchGuard::start(1);
        eta_enqueue(MediaCategory::Video, 1000 * 1024 * 1024);

        let eta = conversion_eta_seconds().expect("1000 MB outstanding");
        assert!(
            (eta - 50.0).abs() < 5.0,
            "the calibrated 20 MB/s must survive the batch reset; got {eta:.1}s \
             (the 2 MB/s compiled-in seed lands near 500s)"
        );

        batch_end();
        reset_ledger_completely();
    }

    /// A pass that converted only images must publish only an image rate.
    /// Reporting a zero (or a seed) for video would overwrite a good stored
    /// value on every images-only scan, which is most of them.
    #[test]
    fn an_images_only_pass_publishes_nothing_about_video() {
        let _lock = global_state_lock();
        batch_end();
        reset_ledger_completely();

        let _guard = ConversionBatchGuard::start(2);
        eta_enqueue(MediaCategory::Image, 20 * 1024 * 1024);
        eta_enqueue(MediaCategory::Video, 500 * 1024 * 1024);
        eta_start(MediaCategory::Image);
        std::thread::sleep(std::time::Duration::from_millis(20));
        eta_complete(MediaCategory::Image, 10 * 1024 * 1024);

        let measured = take_measured_rates();
        assert_eq!(
            measured.len(),
            1,
            "only the sampled category may be published, got {measured:?}"
        );
        assert_eq!(measured[0].0, MediaCategory::Image);

        batch_end();
        reset_ledger_completely();
    }

    /// `take_measured_rates` installs what it returns, so the *next* pass in
    /// this same boot is calibrated without waiting for a restart to re-read
    /// the row.
    #[test]
    fn taking_the_measurement_also_seeds_the_next_pass() {
        let _lock = global_state_lock();
        batch_end();
        reset_ledger_completely();

        {
            let _guard = ConversionBatchGuard::start(1);
            eta_enqueue(MediaCategory::Video, 100 * 1024 * 1024);
            eta_start(MediaCategory::Video);
            std::thread::sleep(std::time::Duration::from_millis(20));
            // ~50 MB over ~20 ms is far above the 2 MB/s seed, and comfortably
            // inside the plausible band however the sleep actually lands.
            eta_complete(MediaCategory::Video, 50 * 1024 * 1024);
            assert_eq!(take_measured_rates().len(), 1, "video was sampled");
        }

        // A fresh pass, no samples of its own: it must quote the measurement,
        // not the seed.
        let _guard = ConversionBatchGuard::start(1);
        eta_enqueue(MediaCategory::Video, 1000 * 1024 * 1024);
        let eta = conversion_eta_seconds().expect("1000 MB outstanding");
        assert!(
            eta < 100.0,
            "the previous pass's measurement must seed this one; got {eta:.1}s \
             (the compiled-in seed lands near 500s)"
        );

        batch_end();
        reset_ledger_completely();
    }

    // ── Calibration persistence (DB half) ───────────────────────────────────
    //
    // These deliberately drive `read_calibration` / `write_calibration` rather
    // than the two public wrappers: the wrappers hold the ledger's std `Mutex`,
    // and a test that awaited while holding it would both trip
    // `clippy::await_holding_lock` and serialise nothing useful.

    async fn calibration_pool() -> sqlx::SqlitePool {
        use std::str::FromStr;
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn stored(pool: &sqlx::SqlitePool, cat: MediaCategory) -> Option<String> {
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = ?")
            .bind(calibration_key(cat))
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    /// The round trip that makes the feature real: what one boot measured, the
    /// next boot reads back.
    #[tokio::test]
    async fn a_measured_rate_survives_a_restart() {
        let pool = calibration_pool().await;
        let image = 45.0 * 1024.0 * 1024.0;
        let video = 3.75 * 1024.0 * 1024.0;

        write_calibration(
            &pool,
            &[(MediaCategory::Image, image), (MediaCategory::Video, video)],
        )
        .await;

        let read = read_calibration(&pool).await;
        assert_eq!(
            read,
            vec![(MediaCategory::Image, image), (MediaCategory::Video, video)],
            "a stored rate must come back exactly, in category order"
        );
    }

    /// A later pass supersedes an earlier one rather than accumulating rows.
    #[tokio::test]
    async fn a_later_pass_overwrites_the_stored_rate() {
        let pool = calibration_pool().await;
        write_calibration(&pool, &[(MediaCategory::Video, 2.0 * 1024.0 * 1024.0)]).await;
        write_calibration(&pool, &[(MediaCategory::Video, 8.0 * 1024.0 * 1024.0)]).await;

        assert_eq!(
            read_calibration(&pool).await,
            vec![(MediaCategory::Video, 8.0 * 1024.0 * 1024.0)]
        );
    }

    /// An images-only pass writes nothing about video, so a video rate measured
    /// earlier is still there afterwards. This is the DB-side counterpart to
    /// `an_images_only_pass_publishes_nothing_about_video`.
    #[tokio::test]
    async fn a_pass_that_measured_nothing_leaves_the_stored_rates_alone() {
        let pool = calibration_pool().await;
        let video = 6.0 * 1024.0 * 1024.0;
        write_calibration(&pool, &[(MediaCategory::Video, video)]).await;

        // The shape `save_throughput_calibration` produces after a pass with no
        // samples at all.
        write_calibration(&pool, &[]).await;

        assert_eq!(
            read_calibration(&pool).await,
            vec![(MediaCategory::Video, video)],
            "an empty measurement must not erase a good stored rate"
        );
    }

    /// A corrupt row degrades that one category to its seed and leaves the
    /// others working — the reason each category gets its own key instead of
    /// one JSON blob.
    #[tokio::test]
    async fn a_corrupt_row_does_not_take_the_other_categories_down() {
        let pool = calibration_pool().await;
        let image = 40.0 * 1024.0 * 1024.0;
        write_calibration(&pool, &[(MediaCategory::Image, image)]).await;

        for junk in ["not-a-number", "NaN", "inf", "0", "-5", "1e15", ""] {
            sqlx::query(
                "INSERT INTO server_settings (key, value) VALUES (?, ?) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(calibration_key(MediaCategory::Video))
            .bind(junk)
            .execute(&pool)
            .await
            .unwrap();

            assert_eq!(
                read_calibration(&pool).await,
                vec![(MediaCategory::Image, image)],
                "video value {junk:?} must be dropped without disturbing image"
            );
        }

        // And the junk is still sitting in the table — reads are non-destructive,
        // so an operator can see what was rejected.
        assert!(stored(&pool, MediaCategory::Video).await.is_some());
    }

    #[test]
    fn batch_guard_finishes_on_panic() {
        let _lock = global_state_lock();
        // Known clean baseline (unpinned + inactive).
        batch_end();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = ConversionBatchGuard::start(5);
            assert!(progress_snapshot().0, "batch is active after start");
            panic!("boom mid-conversion");
        }));

        assert!(
            result.is_err(),
            "panic should propagate out of catch_unwind"
        );
        assert!(
            !progress_snapshot().0,
            "guard's Drop must clear the active flag after a panic (#18)"
        );
    }

    #[test]
    fn batch_guard_explicit_finish_ends_batch() {
        let _lock = global_state_lock();
        batch_end();

        let guard = ConversionBatchGuard::start(3);
        assert!(progress_snapshot().0, "active after start");
        guard.finish();
        assert!(!progress_snapshot().0, "finish() ends the batch");
    }

    #[test]
    fn watchdog_stall_decision() {
        // Active with progress older than the threshold ⇒ stalled (#18).
        let now = 10_000_000i64;
        assert!(is_stalled(true, now - 200_000, now, 100_000));
        // Exactly at the threshold counts as stalled (>=).
        assert!(is_stalled(true, now - 100_000, now, 100_000));
        // Active but recent progress ⇒ healthy.
        assert!(!is_stalled(true, now - 50_000, now, 100_000));
        // Idle (last_progress 0) ⇒ never stalled, even if active races ahead.
        assert!(!is_stalled(false, 0, now, 100_000));
        assert!(!is_stalled(true, 0, now, 100_000));
    }

    #[test]
    fn force_reset_clears_active_and_pin() {
        let _lock = global_state_lock();
        // Simulate a client that declared a batch then vanished mid-flight: the
        // pin + active flag would otherwise starve AI/geo forever.
        batch_start(10);
        progress_add(1);
        assert!(
            progress_snapshot().0,
            "active while a pinned batch is in flight"
        );
        assert!(CONV_PINNED.load(Ordering::Relaxed), "pinned by batch_start");
        let before = CONV_STALL_COUNT.load(Ordering::Relaxed);

        force_reset("test");

        assert!(!progress_snapshot().0, "force_reset clears the active flag");
        assert!(
            !CONV_PINNED.load(Ordering::Relaxed),
            "force_reset clears the client batch pin"
        );
        assert_eq!(
            CONV_STALL_COUNT.load(Ordering::Relaxed),
            before + 1,
            "force_reset records the recovery for telemetry"
        );
        // Leave shared global state clean for the other serialized tests.
        batch_end();
    }

    #[test]
    fn stall_check_disabled_when_threshold_zero() {
        let _lock = global_state_lock();
        batch_start(5); // active, no progress ticks
        assert_eq!(stall_check(0), None, "0 threshold disables the watchdog");
        batch_end();
    }

    #[test]
    fn gpu_attempt_times_out_faster_than_cpu() {
        // The GPU attempt must fail over quickly so a hung hardware encode can't
        // burn the full CPU budget on top of its own before falling back.
        assert!(
            GPU_TRANSCODE_TIMEOUT < CPU_TRANSCODE_TIMEOUT,
            "GPU attempt must be shorter than the CPU fallback"
        );
    }

    #[test]
    fn heartbeat_advances_the_watchdog_liveness_clock() {
        let _lock = global_state_lock();
        batch_start(1); // active; stamps last_progress
                        // Force the liveness clock into the past so a heartbeat visibly advances it.
        CONV_LAST_PROGRESS_MS.store(1, Ordering::Relaxed);
        heartbeat();
        let (last, _, _) = stall_telemetry();
        assert!(
            last > 1,
            "heartbeat() must refresh the watchdog liveness clock"
        );
        // Heartbeat must NOT touch the batch counters — only liveness.
        let (_active, total, done) = progress_snapshot();
        assert_eq!(total, 1, "heartbeat must not change total");
        assert_eq!(done, 0, "heartbeat must not advance done");
        batch_end();
    }

    #[test]
    fn parallelism_single_core_is_fully_serial() {
        // A 1-core box must never spawn more than one encode per lane.
        let p = plan_parallelism(1, false);
        assert_eq!(p.fast_lane, 1);
        assert_eq!(p.video_lane, 1);
        assert_eq!(p.video_threads, 1);
        // Two cores: reserve one for the server ⇒ still fully serial.
        let p = plan_parallelism(2, false);
        assert_eq!(p.fast_lane, 1);
        assert_eq!(p.video_lane, 1);
    }

    #[test]
    fn parallelism_reserves_headroom_for_the_server() {
        // Never hand out every core — ~1/8 is held back for request handling,
        // encryption, and AI/geo so a big import can't wedge the UI.
        for cores in [4usize, 8, 16, 32, 128, 256, 1024] {
            let p = plan_parallelism(cores, false);
            assert!(
                p.fast_lane < cores,
                "{cores} cores: must reserve headroom, got fast_lane={}",
                p.fast_lane
            );
            assert!(p.fast_lane >= 1);
        }
    }

    #[test]
    fn parallelism_scales_up_on_a_many_core_host() {
        // The whole point: a 128-thread threadripper should run dozens of
        // conversions at once, not the old fixed handful.
        let p = plan_parallelism(128, false);
        assert_eq!(p.fast_lane, 112, "128 - 16 reserved");
        assert!(
            p.video_lane > 1,
            "many-core CPU host runs videos in parallel"
        );
        // video_lane * video_threads must stay within the usable budget.
        assert!(p.video_lane * p.video_threads <= 112);

        // And it keeps scaling into the thousands without a hardcoded ceiling.
        let p = plan_parallelism(1024, false);
        assert_eq!(p.fast_lane, 896);
        assert!(p.video_lane >= 100);
    }

    #[test]
    fn parallelism_caps_video_lane_on_gpu() {
        // Hardware encoders only expose a few sessions regardless of core count.
        let p = plan_parallelism(128, true);
        assert_eq!(p.fast_lane, 112, "images still fill the usable cores");
        assert_eq!(p.video_lane, GPU_VIDEO_SESSIONS, "GPU sessions are limited");
    }

    #[test]
    fn parallelism_video_lane_never_exceeds_fast_lane() {
        for cores in [1usize, 2, 4, 8, 16, 64, 1024] {
            for gpu in [false, true] {
                let p = plan_parallelism(cores, gpu);
                assert!(p.video_lane >= 1);
                assert!(p.video_threads >= 1);
                assert!(
                    p.video_lane <= p.fast_lane,
                    "{cores} cores gpu={gpu}: video_lane {} > fast_lane {}",
                    p.video_lane,
                    p.fast_lane
                );
            }
        }
    }

    #[test]
    fn conversion_priority_orders_videos_last() {
        // Fast formats first, slow video transcodes last (#10).
        assert!(
            conversion_priority(MediaCategory::Image) < conversion_priority(MediaCategory::Video)
        );
        assert!(
            conversion_priority(MediaCategory::Audio) < conversion_priority(MediaCategory::Video)
        );
    }

    #[test]
    fn pinned_batch_keeps_denominator_stable() {
        let _lock = global_state_lock();
        // Simulate the web upload loop declaring a 4-file convertible batch and
        // the inline upload path converting them one at a time. The denominator
        // must stay at 4 the whole way (#11), never tracking one ahead.
        batch_start(4);
        let (active, total, done) = progress_snapshot();
        assert!(active);
        assert_eq!(total, 4);
        assert_eq!(done, 0);

        for expected_done in 1..=4 {
            // Each inline conversion would self-register +1; while pinned this
            // must NOT grow the total.
            progress_add(1);
            assert_eq!(progress_snapshot().1, 4, "total must stay pinned at 4");
            progress_finish_one();
            assert_eq!(progress_snapshot().2, expected_done);
        }

        // A background pass firing mid-batch must not reset the denominator.
        progress_start(99);
        assert_eq!(
            progress_snapshot().1,
            4,
            "progress_start ignored while pinned"
        );

        batch_end();
        let (active, _, _) = progress_snapshot();
        assert!(!active, "batch_end finalizes the banner");

        // After unpin, the normal per-pass path works again.
        progress_start(7);
        assert_eq!(progress_snapshot().1, 7);
        progress_finish();
        assert!(!progress_snapshot().0);
    }

    #[test]
    fn conversion_priority_sorts_a_mixed_batch_fast_first() {
        let mut cats = vec![
            MediaCategory::Video,
            MediaCategory::Image,
            MediaCategory::Video,
            MediaCategory::Audio,
            MediaCategory::Image,
        ];
        // Stable sort preserves discovery order within each tier.
        cats.sort_by_key(|c| conversion_priority(*c));
        assert_eq!(
            cats,
            vec![
                MediaCategory::Image,
                MediaCategory::Image,
                MediaCategory::Audio,
                MediaCategory::Video,
                MediaCategory::Video,
            ]
        );
    }
}
