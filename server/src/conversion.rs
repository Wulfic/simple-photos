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
use std::sync::OnceLock;
use std::time::Duration;

use axum::Json;
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
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
            let args = crate::transcode::ffmpeg_gpu::build_video_transcode_args(input, output, hw);
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
    let threads_arg = video_threads.map(|n| n.max(1)).unwrap_or(0).to_string();
    cmd.args([
        "-y",
        "-i",
        input,
        "-threads",
        &threads_arg,
        "-vf",
        "scale=trunc(iw*sar/2)*2:trunc(ih/2)*2,setsar=1",
        "-c:v",
        "libx264",
        "-preset",
        "medium",
        "-crf",
        "20",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-movflags",
        "+faststart",
        output,
    ])
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

/// Estimated seconds remaining for the active conversion batch (item #4).
/// Uses the same throughput estimator as the encryption banner
/// ([`crate::status::progress_math`]); `None` until at least one item finishes
/// (no throughput sample yet) or when idle.
pub fn conversion_eta_seconds() -> Option<f64> {
    let (active, total, done) = progress_snapshot();
    if !active {
        return None;
    }
    let started = CONV_STARTED_MS.load(Ordering::Relaxed);
    if started == 0 {
        return None;
    }
    let elapsed = (now_ms() - started) as f64 / 1000.0;
    let remaining = (total - done).max(0);
    let (_done, eta) = crate::status::progress_math(total, remaining, elapsed);
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
        assert!(p.video_lane > 1, "many-core CPU host runs videos in parallel");
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
