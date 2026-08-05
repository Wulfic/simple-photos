//! Producing the renditions the ladder plans (#49).
//!
//! [`rung_queue`](super::rung_queue) says which videos owe a rung,
//! [`ladder`](super::ladder) says what shape it should be, and
//! [`renditions`](super::renditions) records the result. This module is the part
//! that actually spends CPU: decrypt → probe → plan → transcode → encrypt →
//! record, once per candidate.
//!
//! # The probe is the only source of geometry
//!
//! Every dimension used here comes from probing the file about to be encoded.
//! `photos.width`/`height` are used by the SQL prefilter and nowhere else, for
//! two measured reasons (see [`rung_queue`](super::rung_queue) for the numbers):
//! 58 live videos have no recorded geometry at all, and for a large part of the
//! library the stored pair is transposed relative to what ffprobe reports.
//! Transposition is harmless to a rule keyed on `min(w, h)` and fatal to a
//! `scale=W:H`, which would squash a landscape frame into a portrait box.
//!
//! # Why this is a separate pass and not part of conversion
//!
//! A first-pass conversion is what makes a video playable at all; a rung is a
//! convenience. Running them together would mean a user waiting on a 4K
//! downscale before seeing any video, which is the one outcome `todo.md`
//! forbids. So this runs after `run_conversion_pass`, and never beside it — see
//! [`should_defer_sweep`], which enforces by policy what three call sites used
//! to arrange by luck.
//!
//! # How wide the sweep runs (#49 cost control)
//!
//! At the **video lane's width**, from the same
//! [`ConversionParallelism`](crate::conversion::ConversionParallelism) plan the
//! conversion pass uses. It was serial until 2026-08-04; the reasons for the
//! change, and the reasons it is not wider, are both worth keeping:
//!
//! * A serial sweep was already *budgeting* itself for a parallel one. Each
//!   encode is capped at `video_threads`, which is `usable / video_lane` — a
//!   share sized for `video_lane` concurrent encodes. Running exactly one of
//!   them used `1 / video_lane` of the box it had reserved: on a 128-thread
//!   host, 8 threads out of 112 usable, while a 4K backlog drained.
//! * There is no serial fix for that. Handing the one encode all 112 threads is
//!   what `CPU_VIDEO_THREADS_TARGET` exists to refuse — libx264 thread scaling
//!   plateaus in the single digits, so the extra cores idle inside ffmpeg
//!   instead of idling outside it.
//! * `video_lane` is **1** for any host with fewer than ~24 threads
//!   (`usable / 8`, integer division), so on ordinary hardware this change is a
//!   provable no-op and the sweep stays exactly as serial as it was. Cores get
//!   spent only where they exist, and `SIMPLE_PHOTOS_CONVERSION_JOBS` pins the
//!   budget for an operator who disagrees — one knob for both passes, which is
//!   the *other* half of this fix: `transcode_to_rung` used to plan its own
//!   threads from `plan_parallelism` and never saw that variable at all.
//!
//! What it does **not** fix, stated so the next person measuring a slow drain
//! looks in the right place: the sweep is also bounded by
//! [`SWEEP_CANDIDATE_LIMIT`] (16 files) and by how often autoscan calls it
//! (~hourly when idle). Against the live 114-file backlog those two dominate,
//! and on a box where `video_lane == 1` the lane width changes nothing at all.
//! Widening either of those is a separate decision with a disk-I/O cost
//! ([[idle-disk-thrash-investigation]]), not a lane-width one.
//!
//! The cost bought here is concurrent scratch: every candidate materialises its
//! **whole** decrypted source before probing it, so a lane `N` wide holds `N`
//! plaintext 4K videos under `.rendition_tmp` at once. First-pass conversion
//! does not pay that (it converts files already in the clear on disk), which is
//! why the two lanes are not simply interchangeable.
//!
//! # Failure is expected and must be bounded
//!
//! Attempts are charged before ffmpeg starts, never after it fails — a file that
//! OOMs or hard-kills the encoder never reaches an error handler, and it is
//! exactly the file that must stop being retried. See
//! `036_video_rendition_attempts.sql`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use futures_util::stream::{self, StreamExt};
use uuid::Uuid;

use crate::blobs::{chunked, storage};
use crate::conversion::ConversionParallelism;

use super::ladder::{self, TIER_1080_SHORT_EDGE};
use super::probe;
use super::renditions::{upsert_rendition, StoredRendition};
use super::rung_queue::{self, RungCandidate, MAX_RUNG_ATTEMPTS};

/// Candidates fetched per sweep.
///
/// A ceiling on the query, not a target: [`SWEEP_TIME_BUDGET`] usually stops the
/// sweep first. It exists so a boot against the 114-candidate live backlog reads
/// a bounded number of rows rather than the whole set it will not finish.
const SWEEP_CANDIDATE_LIMIT: i64 = 16;

/// Wall-clock budget for one sweep, checked **between** files.
///
/// Never mid-encode: killing a 4K transcode at 90% to respect a budget wastes
/// everything it has done and charges an attempt for it. A single file may
/// therefore overrun this; the budget bounds how many files a sweep starts, not
/// how long the last one runs.
const SWEEP_TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Serialises sweeps. Autoscan calls this from several sites and an interval
/// tick can land while the previous sweep is still encoding; two concurrent
/// sweeps would select the same candidates and burn two 4K encodes to produce
/// one rendition.
///
/// Still required now that a *single* sweep runs several encodes at once: this
/// guards against two sweeps selecting the same rows, which the in-sweep lane
/// cannot, because `find_rung_candidates` is one query per sweep.
static SWEEP_RUNNING: AtomicBool = AtomicBool::new(false);

/// How wide one sweep runs and how many threads each encode gets.
///
/// Both halves come from one [`ConversionParallelism`] plan — see the module
/// doc for why that matters and [`plan_sweep_budget`] for the derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepBudget {
    /// Concurrent rendition encodes.
    pub lane: usize,
    /// `-threads` handed to each of them.
    pub threads: usize,
}

/// Derive a sweep's budget from a conversion parallelism plan.
///
/// The ladder deliberately reuses the **video** lane rather than defining a
/// third one: a rendition encode is a video transcode, subject to the same
/// libx264 thread plateau on the CPU path and the same hardware session cap on
/// the GPU path. A ladder-specific constant would be a fourth copy of a number
/// `conversion.rs` already owns, and the cross-cutting rule in `todo.md` about
/// two derivations of one list applies to budgets too.
pub fn plan_sweep_budget(plan: ConversionParallelism) -> SweepBudget {
    SweepBudget {
        lane: plan.video_lane.max(1),
        threads: plan.video_threads.max(1),
    }
}

/// Should this sweep stand down, and if so why?
///
/// Returns the log reason, or `None` to proceed. Pure so the ordering can be
/// asserted without a live migration or a held lock.
///
/// `conversion_running` is the arm added with the lane (#49 cost control), and
/// it is not merely politeness. The sweep and the conversion pass each open a
/// video lane `plan.video_lane` wide; on the GPU path that width *is* the
/// hardware session cap, so two at once doubles it, and on the CPU path it
/// spends the core reserve that keeps the UI responsive. Sequencing at the
/// three autoscan sites used to make this unlikely — but `upload.rs` and
/// `scan.rs` kick `run_conversion_pass` with no such sequencing, so an upload
/// landing mid-sweep genuinely raced it. A deferred sweep costs one hour of a
/// convenience feature; the collision costs failed encodes on hardware the user
/// is actively waiting on.
pub fn should_defer_sweep(
    migration_active: bool,
    conversion_running: bool,
) -> Option<&'static str> {
    if migration_active {
        // Encryption comes first. A ladder encode competing with the encryption
        // backlog delays photos becoming *viewable at all* in order to add a
        // quality option to a video the user can already play.
        Some("encryption migration active")
    } else if conversion_running {
        Some("conversion pass running")
    } else {
        None
    }
}

/// What one sweep did. Returned for logging and asserted by the tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepOutcome {
    pub examined: usize,
    pub produced: usize,
    pub not_needed: usize,
    pub failed: usize,
    pub skipped: usize,
    /// Of `examined`/`produced`, how many came from the #46 codec backfill
    /// (registered videos re-examined for a non-native codec) rather than the
    /// resolution ladder. Split out only so the logs can tell "the library is
    /// still draining its one-off codec pass" from "new 4K uploads need rungs".
    pub backfill_examined: usize,
    pub backfill_produced: usize,
    /// The most encodes that were ever in flight at once across the sweep.
    ///
    /// Logged, because "the lane is 8 wide" and "8 encodes actually ran
    /// together" are different claims and only the second is evidence. It is
    /// also what the concurrency test asserts: a serial driver reports 1 no
    /// matter how wide the budget says it is.
    pub peak_concurrency: usize,
}

/// A file that must not outlive the step that made it.
///
/// The decrypted source of a 4K video is gigabytes of plaintext, and the whole
/// point of encrypted mode is that it does not sit on disk. Cleanup lives in
/// `Drop` rather than at the end of the happy path because every failure branch
/// here returns early, and a `?` that skipped the cleanup would leak the user's
/// decrypted video permanently.
struct ScratchFile(PathBuf);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = ?self.0, "failed to remove rendition scratch file: {e}");
            }
        }
    }
}

impl ScratchFile {
    fn path(&self) -> &Path {
        &self.0
    }
}

/// Run one ladder sweep, if one is not already running.
///
/// Mirrors `server_migrate::auto_migrate_after_scan`: it loads its own key, does
/// nothing when there is no work, and never returns an error to its caller —
/// this is background maintenance, and a failure to produce a rendition must not
/// affect the scan that triggered it.
pub async fn generate_rungs_after_scan(
    pool: sqlx::SqlitePool,
    storage_root: PathBuf,
    jwt_secret: String,
) {
    if let Some(reason) = should_defer_sweep(
        crate::photos::server_migrate::migration_active().await,
        crate::ingest::conversion_pass_running(),
    ) {
        tracing::debug!("[LADDER] {reason} — deferring rung sweep");
        return;
    }

    if SWEEP_RUNNING.swap(true, Ordering::AcqRel) {
        tracing::debug!("[LADDER] sweep already running — skipping this trigger");
        return;
    }

    let outcome = run_sweep(&pool, &storage_root, &jwt_secret).await;

    SWEEP_RUNNING.store(false, Ordering::Release);

    if outcome.examined > 0 {
        tracing::info!(
            examined = outcome.examined,
            produced = outcome.produced,
            not_needed = outcome.not_needed,
            failed = outcome.failed,
            skipped = outcome.skipped,
            backfill_examined = outcome.backfill_examined,
            backfill_produced = outcome.backfill_produced,
            peak_concurrency = outcome.peak_concurrency,
            "[LADDER] rung sweep complete"
        );
    }
}

/// The sweep body, separated from the re-entrancy guard so tests can drive it
/// directly without racing a static.
///
/// Plans the budget from the host and delegates. The split exists because the
/// host's own `video_lane` is **1** on any ordinary dev machine (see the module
/// doc), so a concurrency test calling this would assert nothing on the very
/// machines it runs on — it must state the width it is testing.
pub async fn run_sweep(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    jwt_secret: &str,
) -> SweepOutcome {
    // `detect_parallelism`, not `plan_parallelism`: the former honours
    // `SIMPLE_PHOTOS_CONVERSION_JOBS`. Planned once here, and both halves travel
    // together from this point — the lane width the driver uses and the
    // `-threads` each encode gets are two fields of one plan, never two
    // derivations.
    let gpu = crate::conversion::active_hwaccel()
        .map(|h| h.is_gpu())
        .unwrap_or(false);
    let budget = plan_sweep_budget(crate::conversion::detect_parallelism(gpu));
    run_sweep_with_budget(pool, storage_root, jwt_secret, budget).await
}

/// [`run_sweep`] with the concurrency budget supplied rather than detected.
pub async fn run_sweep_with_budget(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    jwt_secret: &str,
    budget: SweepBudget,
) -> SweepOutcome {
    let mut outcome = SweepOutcome::default();

    // Reclaim the bytes of any renditions deleted since the last sweep, first
    // and unconditionally: it is cheap, independent of whether there is any rung
    // work, and a deleted 4K source's rendition is hundreds of megabytes leaking
    // until this runs. Its own summary is logged inside; it never errors out.
    crate::transcode::orphan_sweep::sweep_orphaned_rendition_blobs(pool, storage_root).await;

    // Resolution rungs first: these deliver a quality a client is actively
    // waiting on. The candidate set is deliberately narrow (oversized + unknown
    // geometry).
    let rung_candidates = match rung_queue::find_rung_candidates(pool, SWEEP_CANDIDATE_LIMIT).await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[LADDER] failed to select rung candidates: {e}");
            return outcome;
        }
    };

    // The #46 codec backfill: known-small registered videos whose container was
    // trusted by extension and never re-examined. Fetched even when there are no
    // rungs, because after the ladder has drained this is the only work left and
    // it must still make progress — but its failure is non-fatal to the rung
    // pass, so a query error here is logged and treated as "no backfill work".
    let backfill_candidates =
        match rung_queue::find_codec_backfill_candidates(pool, SWEEP_CANDIDATE_LIMIT).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("[LADDER] failed to select codec-backfill candidates: {e}");
                Vec::new()
            }
        };

    // Nothing to do at all — return BEFORE loading the key. Unwrapping it is a
    // KDF, and paying it on every idle autoscan tick (the steady state, once the
    // library has drained) is exactly the needless work this pass must not add.
    if rung_candidates.is_empty() && backfill_candidates.is_empty() {
        return outcome;
    }

    // Loaded once per sweep, not once per file: it is the same key every time.
    // `None` is legitimate — an unencrypted install has no stored key, and its
    // candidates are served from `file_path` instead.
    let key = match crate::crypto::load_wrapped_key(pool, jwt_secret).await {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("[LADDER] failed to load encryption key: {e}");
            None
        }
    };

    // `blind` are rung candidates the prefilter selected without usable stored
    // geometry (the ~58 rows with width/height <= 0): they are probed to find
    // out whether they even need a rung, rather than because they are known
    // oversized. Surfacing the split makes a sweep that spends its budget mostly
    // on speculative probes legible instead of looking like real rung work.
    let blind = rung_candidates
        .iter()
        .filter(|c| !c.geometry_is_known())
        .count();

    tracing::info!(
        rungs = rung_candidates.len(),
        blind,
        backfill = backfill_candidates.len(),
        lane = budget.lane,
        threads = budget.threads,
        "[LADDER] starting video rendition sweep"
    );

    // One wall-clock budget spans BOTH phases: rungs get first call on it, and
    // the backfill runs only with whatever is left. So a busy rung queue can
    // starve the backfill for a sweep, which is the correct priority — a rendition
    // a client plays outranks re-examining a file's codec — and the backfill is
    // bounded work that resumes next sweep regardless.
    let started = std::time::Instant::now();
    process_phase(
        pool,
        storage_root,
        key.as_ref(),
        rung_candidates,
        started,
        budget,
        false,
        &mut outcome,
    )
    .await;
    process_phase(
        pool,
        storage_root,
        key.as_ref(),
        backfill_candidates,
        started,
        budget,
        true,
        &mut outcome,
    )
    .await;

    outcome
}

/// Concurrent tallies for one phase.
///
/// The counters were plain `usize` increments on a `&mut SweepOutcome` while the
/// driver was serial; a lane wider than one needs them shared, and the peak
/// gauge only exists once there is a lane at all.
#[derive(Default)]
struct PhaseTally {
    examined: AtomicUsize,
    produced: AtomicUsize,
    not_needed: AtomicUsize,
    failed: AtomicUsize,
    skipped: AtomicUsize,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    /// Latches the first "we stopped starting files" log. `for_each_concurrent`
    /// keeps pulling from the stream after the budget is spent — each remaining
    /// item becomes a cheap no-op — so without this the reason is logged once
    /// per deferred candidate.
    stopped_logged: AtomicBool,
}

/// Drain one phase's candidate list through [`generate_one`], sharing the sweep's
/// wall-clock budget. `is_backfill` only selects which counters to bump and the
/// log label — the encode path is identical, which is the whole point of routing
/// #46's backfill through the ladder's existing pipeline.
#[allow(clippy::too_many_arguments)]
async fn process_phase(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    key: Option<&[u8; 32]>,
    candidates: Vec<RungCandidate>,
    started: std::time::Instant,
    budget: SweepBudget,
    is_backfill: bool,
    outcome: &mut SweepOutcome,
) {
    let phase = if is_backfill { "backfill" } else { "rung" };
    let tally = PhaseTally::default();

    stream::iter(candidates)
        .for_each_concurrent(budget.lane, |candidate| {
            let tally = &tally;
            async move {
                // Both stop conditions are evaluated HERE rather than in a
                // driver loop, and that is still "between files":
                // `for_each_concurrent` polls the stream only as a lane slot
                // frees, so nothing already encoding is ever interrupted. The
                // budget must not cut a 4K transcode off at 90% — that wastes
                // everything it has done and charges an attempt for it.
                let stop = if started.elapsed() >= SWEEP_TIME_BUDGET {
                    Some("sweep budget reached")
                } else if crate::ingest::conversion_pass_running() {
                    // Checked per file, not just at sweep start: an upload can
                    // kick a conversion pass at any point during a sweep, and
                    // from that moment on the two lanes would be competing for
                    // one core reserve / one set of GPU sessions.
                    Some("a conversion pass started")
                } else {
                    None
                };
                if let Some(reason) = stop {
                    if !tally.stopped_logged.swap(true, Ordering::AcqRel) {
                        tracing::info!(
                            phase,
                            elapsed_secs = started.elapsed().as_secs(),
                            "[LADDER] {reason} — remaining candidates deferred to the next sweep"
                        );
                    }
                    return;
                }

                tally.examined.fetch_add(1, Ordering::Relaxed);
                let now = tally.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
                tally.peak.fetch_max(now, Ordering::AcqRel);
                let file_start = std::time::Instant::now();

                let result =
                    generate_one(pool, storage_root, key, &candidate, budget.threads).await;

                tally.in_flight.fetch_sub(1, Ordering::AcqRel);

                match result {
                    Ok(Verdict::Produced { short_edge }) => {
                        tally.produced.fetch_add(1, Ordering::Relaxed);
                        tracing::info!(
                            phase,
                            photo_id = %candidate.photo_id,
                            filename = %candidate.filename,
                            short_edge,
                            elapsed_secs = file_start.elapsed().as_secs(),
                            "[LADDER] produced video rendition"
                        );
                    }
                    Ok(Verdict::NotNeeded) => {
                        tally.not_needed.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!(
                            phase,
                            photo_id = %candidate.photo_id,
                            filename = %candidate.filename,
                            "[LADDER] no rung owed"
                        );
                    }
                    Ok(Verdict::Skipped(reason)) => {
                        tally.skipped.fetch_add(1, Ordering::Relaxed);
                        // Info, not warn: every skip reason is environmental and
                        // self-resolving (encryption pending, key absent, bytes
                        // not where the row says). None needs an operator
                        // tonight, and the encryption-backlog case would
                        // otherwise warn once per candidate per sweep for as
                        // long as the backlog exists.
                        tracing::info!(
                            phase,
                            photo_id = %candidate.photo_id,
                            filename = %candidate.filename,
                            "[LADDER] skipped: {reason}"
                        );
                    }
                    Err(e) => {
                        tally.failed.fetch_add(1, Ordering::Relaxed);
                        tracing::error!(
                            phase,
                            photo_id = %candidate.photo_id,
                            filename = %candidate.filename,
                            elapsed_secs = file_start.elapsed().as_secs(),
                            "[LADDER] rung generation failed: {e}"
                        );
                    }
                }
            }
        })
        .await;

    let examined = tally.examined.load(Ordering::Relaxed);
    let produced = tally.produced.load(Ordering::Relaxed);
    outcome.examined += examined;
    outcome.produced += produced;
    outcome.not_needed += tally.not_needed.load(Ordering::Relaxed);
    outcome.failed += tally.failed.load(Ordering::Relaxed);
    outcome.skipped += tally.skipped.load(Ordering::Relaxed);
    if is_backfill {
        outcome.backfill_examined += examined;
        outcome.backfill_produced += produced;
    }
    // Max, not sum: the two phases run one after the other, so the sweep's peak
    // is the higher of theirs, never their total.
    outcome.peak_concurrency = outcome
        .peak_concurrency
        .max(tally.peak.load(Ordering::Relaxed));
}

/// The outcome for one candidate that did not error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A rung was encoded, stored and recorded.
    Produced { short_edge: i64 },
    /// The probe found the source at or below the tier. Terminal.
    NotNeeded,
    /// Nothing could be attempted (no key, missing bytes). Not charged as an
    /// attempt, because no encode was tried and the cause is environmental.
    Skipped(String),
}

/// Decrypt → probe → plan → transcode → encrypt → record, for one video.
///
/// `encode_threads` is the per-encode `-threads` budget from the sweep's
/// [`SweepBudget`]; several of these run concurrently, so it is a share of the
/// host, not the whole of it.
pub async fn generate_one(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    key: Option<&[u8; 32]>,
    candidate: &RungCandidate,
    encode_threads: usize,
) -> Result<Verdict, String> {
    let scratch_dir = storage_root.join(".rendition_tmp");
    tokio::fs::create_dir_all(&scratch_dir)
        .await
        .map_err(|e| format!("create rendition scratch dir: {e}"))?;

    // ── Materialise plaintext ────────────────────────────────────────────────
    // `_decrypted` is bound (not `_`) so it lives to the end of this function:
    // `let _ = ScratchFile(..)` drops immediately and would delete the file
    // before ffmpeg ever opened it.
    let (source_path, _decrypted): (PathBuf, Option<ScratchFile>) =
        match candidate.encrypted_blob_id.as_deref() {
            Some(blob_id) => {
                let Some(key) = key else {
                    return Ok(Verdict::Skipped(
                        "photo is encrypted but no key is available".into(),
                    ));
                };
                let Some(enc_path) = blob_file_path(pool, storage_root, blob_id).await? else {
                    return Ok(Verdict::Skipped(format!(
                        "blob {blob_id} has no stored path"
                    )));
                };
                if !tokio::fs::try_exists(&enc_path).await.unwrap_or(false) {
                    return Ok(Verdict::Skipped(format!(
                        "blob {blob_id} is recorded but missing on disk"
                    )));
                }

                let dst = scratch_dir.join(format!("{}.src.mp4", candidate.photo_id));
                let scratch = ScratchFile(dst.clone());
                let key_copy = *key;
                let src = enc_path.clone();
                let out = dst.clone();
                tokio::task::spawn_blocking(move || {
                    chunked::decrypt_blob_file_to_file(&key_copy, &src, &out)
                })
                .await
                .map_err(|e| format!("decrypt task panicked: {e}"))?
                .map_err(|e| format!("decrypt source blob: {e}"))?;

                (dst, Some(scratch))
            }
            None => {
                // An encrypted install with a photo that has no blob yet: this
                // row is in the encryption backlog (2,494 of them live). Its
                // bytes are about to move, and a rendition recorded as a
                // `file_path` now would be one neither client can play —
                // both play from blobs. Wait for the migration.
                if key.is_some() {
                    return Ok(Verdict::Skipped(
                        "photo is still awaiting encryption; deferring its rung".into(),
                    ));
                }
                if candidate.file_path.is_empty() {
                    return Ok(Verdict::Skipped(
                        "photo has neither an encrypted blob nor a file path".into(),
                    ));
                }
                let path = storage_root.join(&candidate.file_path);
                if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
                    return Ok(Verdict::Skipped(format!(
                        "source file is missing: {}",
                        candidate.file_path
                    )));
                }
                (path, None)
            }
        };

    // ── Probe: the only geometry this function trusts ────────────────────────
    let info = probe::probe_video_stream(&source_path)
        .await
        .map_err(|e| format!("probe source: {e}"))?;

    if info.width <= 0 || info.height <= 0 {
        return Ok(Verdict::Skipped(
            "probe returned no usable dimensions".into(),
        ));
    }

    // Bounded by ffprobe's decode-health window, so this costs a few seconds
    // rather than a full decode of a 4K file. `None` on failure means "not
    // checked", which `source_rung_is_offerable` treats as the pre-#46 level of
    // confidence — never as evidence of health.
    let health = probe::probe_decode_health(&source_path).await.ok();
    let offerable = ladder::source_rung_is_offerable(&info, health.as_ref());

    let plan = ladder::plan_ladder(info.width, info.height, offerable);
    let Some(rung) = plan.iter().find(|r| !r.is_source).copied() else {
        // Either the source is at or below the tier, or the only rung planned
        // was the source itself. Both are terminal: no encode will ever be owed
        // for this file as it stands, and recording that is what stops the
        // deliberately-wide prefilter from re-selecting it forever.
        rung_queue::mark_rung_not_needed(pool, &candidate.photo_id, TIER_1080_SHORT_EDGE)
            .await
            .map_err(|e| format!("record not-needed verdict: {e}"))?;
        return Ok(Verdict::NotNeeded);
    };

    // ── Charge the attempt, then encode ──────────────────────────────────────
    let attempt = rung_queue::begin_attempt(pool, &candidate.photo_id, rung.short_edge)
        .await
        .map_err(|e| format!("charge rung attempt: {e}"))?;
    if attempt > MAX_RUNG_ATTEMPTS {
        // The candidate query filters these out, so reaching this means a sweep
        // raced another writer. Retiring here keeps the cap true regardless.
        return Ok(Verdict::Skipped(format!(
            "attempt {attempt} exceeds the cap of {MAX_RUNG_ATTEMPTS}"
        )));
    }

    let out_path = scratch_dir.join(format!("{}.{}.mp4", candidate.photo_id, rung.short_edge));
    let encoded = ScratchFile(out_path.clone());

    if let Err(e) = crate::conversion::transcode_to_rung(
        &source_path,
        &out_path,
        (rung.width, rung.height),
        encode_threads,
    )
    .await
    {
        rung_queue::record_failure(pool, &candidate.photo_id, rung.short_edge, attempt, &e)
            .await
            .map_err(|e| format!("record rung failure: {e}"))?;
        return Err(e);
    }

    // ── Store ────────────────────────────────────────────────────────────────
    // A rendition is stored in whatever mode its parent photo is, which the
    // materialise step above has already narrowed to exactly two cases: an
    // encrypted photo with a key, or an unencrypted install. The mixed case
    // (encrypted install, photo still in the backlog) returned Skipped there.
    let stored = match (key, candidate.encrypted_blob_id.is_some()) {
        (Some(key), true) => {
            store_encrypted(pool, storage_root, key, candidate, &rung, encoded.path()).await?
        }
        _ => store_plaintext(storage_root, candidate, &rung, encoded.path()).await?,
    };

    if let Err(e) = upsert_rendition(pool, &stored).await {
        let msg = format!("record produced rendition: {e}");
        rung_queue::record_failure(pool, &candidate.photo_id, rung.short_edge, attempt, &msg)
            .await
            .map_err(|e| format!("record rung failure: {e}"))?;
        return Err(msg);
    }

    // The source rung, recorded only now. A picker with a 1080p entry and no
    // "original" is worse than no picker: the user can no longer reach the
    // quality they already had. It is recorded *after* the rung it accompanies
    // so that state can never exist, not even briefly.
    if offerable {
        let source_row = StoredRendition {
            photo_id: candidate.photo_id.clone(),
            short_edge: ladder::short_edge(info.width, info.height),
            width: info.width,
            height: info.height,
            is_source: 1,
            // Points at the bytes the PHOTO already owns — this row is a second
            // reference, never a copy. `037`'s orphan trigger excludes
            // `is_source` rows for exactly this reason.
            blob_id: candidate.encrypted_blob_id.clone(),
            file_path: candidate
                .encrypted_blob_id
                .is_none()
                .then(|| candidate.file_path.clone()),
            codec: Some(info.codec.clone()),
            bitrate: None,
            size_bytes: tokio::fs::metadata(&source_path)
                .await
                .map(|m| m.len() as i64)
                .unwrap_or(0),
        };
        if let Err(e) = upsert_rendition(pool, &source_row).await {
            // Not fatal: the 1080p rung exists and plays. Logged loudly because
            // the picker is now missing its top entry, which looks like the
            // ladder downgraded the user's video.
            tracing::error!(
                photo_id = %candidate.photo_id,
                "[LADDER] produced a rung but failed to record the source rendition: {e}"
            );
        }
    }

    // #46 corrupt-file honesty: if the source did not decode cleanly, this rung
    // is a *salvage* — ffmpeg kept the frames it could read and dropped the rest,
    // so the rendition is shorter than the source. Surface the loss in the audit
    // log rather than handing the user a silently-truncated video (the natural
    // first consumer of #45's failure events). A merely non-native but clean
    // source — a healthy HEVC — is a lossless re-encode and says nothing here.
    if let Some(h) = health.as_ref().filter(|h| !h.is_clean()) {
        crate::audit::log_background(
            pool,
            crate::audit::AuditEvent::MediaConvert,
            Some(serde_json::json!({
                "filename": candidate.filename,
                "category": "video",
                "short_edge": rung.short_edge,
                "salvage": true,
                "decode_errors": h.error_count,
                "first_error": h.first_error,
                "note": "corrupt source salvaged; rendition is shorter than the original",
                "origin": "rung_backfill",
            })),
        );
    }

    Ok(Verdict::Produced {
        short_edge: rung.short_edge,
    })
}

/// Resolve a blob's on-disk path from its recorded `storage_path`.
///
/// Read from the row rather than recomputed with [`storage::blob_path`]: the
/// stored path is what the serve layer uses, and a rendition pass that derives
/// its own would silently diverge for any blob written before a path-scheme
/// change.
async fn blob_file_path(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    blob_id: &str,
) -> Result<Option<PathBuf>, String> {
    let rel: Option<String> = sqlx::query_scalar("SELECT storage_path FROM blobs WHERE id = ?")
        .bind(blob_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("look up blob {blob_id}: {e}"))?;

    Ok(rel.map(|r| storage_root.join(r)))
}

/// Encrypt the encoded rung into a new blob and register it.
///
/// Chunked (`SPCHNKB2`) unconditionally, never `crypto::encrypt`: a v1 encrypt
/// of a 1080p video holds roughly five times the file on the heap, which is the
/// OOM that migration `024` exists to prevent.
async fn store_encrypted(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    key: &[u8; 32],
    candidate: &RungCandidate,
    rung: &ladder::Rendition,
    encoded: &Path,
) -> Result<StoredRendition, String> {
    let size_bytes = tokio::fs::metadata(encoded)
        .await
        .map(|m| m.len())
        .map_err(|e| format!("stat encoded rung: {e}"))?;

    // The envelope a client decrypts before playing. `mime_type` is the only
    // load-bearing field — `decryptPhotoBlobToBlob` uses it as the Blob's type,
    // and a wrong value hands the player bytes it will not decode. The rest is
    // informational, and the dimensions are the RUNG's, not the source's.
    let meta = serde_json::json!({
        "v": chunked::FORMAT_V2,
        "filename": candidate.filename,
        "mime_type": "video/mp4",
        "media_type": "video",
        "width": rung.width,
        "height": rung.height,
        "chunk_size": chunked::CHUNK_SIZE,
        "data_len": size_bytes,
    });
    let meta_json =
        serde_json::to_vec(&meta).map_err(|e| format!("serialize rendition envelope: {e}"))?;

    let blob_id = Uuid::new_v4().to_string();
    let blob_abs = storage::blob_path(storage_root, &candidate.user_id, &blob_id);
    if let Some(parent) = blob_abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create rendition blob directory: {e}"))?;
    }
    let blob_rel = storage::relative_path(&candidate.user_id, &blob_id);

    let key_copy = *key;
    let src = encoded.to_path_buf();
    let dst = blob_abs.clone();
    let result = tokio::task::spawn_blocking(move || {
        chunked::encrypt_file_chunked(&key_copy, &src, &dst, &meta_json)
    })
    .await
    .map_err(|e| format!("rendition encrypt task panicked: {e}"))?
    .map_err(|e| format!("encrypt rendition: {e}"))?;

    // `content_hash` stays NULL, as thumbnail blobs already do. It is the dedup
    // key: a rendition sharing content with a photo's own blob would let the
    // dedup path link the two, and a later "reuse this existing blob" would hand
    // a photo its own downscale as the original.
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, \
         storage_path, content_hash, blob_format) \
         VALUES (?, ?, 'video', ?, ?, ?, ?, NULL, 1)",
    )
    .bind(&blob_id)
    .bind(&candidate.user_id)
    .bind(size_bytes as i64)
    .bind(hex::encode(result.blob_sha256))
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(&blob_rel)
    .execute(pool)
    .await
    .map_err(|e| {
        // The bytes are already on disk; without a row nothing will ever find
        // them again, so say so rather than letting it read as a lost encode.
        tracing::error!(
            blob_id = %blob_id,
            path = %blob_rel,
            "[LADDER] rendition encrypted but its blob row failed to insert — orphaned bytes"
        );
        format!("insert rendition blob row: {e}")
    })?;

    Ok(StoredRendition {
        photo_id: candidate.photo_id.clone(),
        short_edge: rung.short_edge,
        width: rung.width,
        height: rung.height,
        is_source: 0,
        blob_id: Some(blob_id),
        file_path: None,
        codec: Some("h264".into()),
        bitrate: None,
        size_bytes: size_bytes as i64,
    })
}

/// Move the encoded rung into place for an unencrypted install.
async fn store_plaintext(
    storage_root: &Path,
    candidate: &RungCandidate,
    rung: &ladder::Rendition,
    encoded: &Path,
) -> Result<StoredRendition, String> {
    let rel = format!(
        "renditions/{}/{}.{}.mp4",
        candidate.user_id, candidate.photo_id, rung.short_edge
    );
    let abs = storage_root.join(&rel);
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create rendition directory: {e}"))?;
    }

    // Copy rather than rename: the scratch file and the storage root can be on
    // different filesystems (a scratch dir under the root is not guaranteed to
    // share a mount with it), and `rename` fails across devices. The scratch
    // copy is removed by its `ScratchFile` guard either way.
    tokio::fs::copy(encoded, &abs)
        .await
        .map_err(|e| format!("place rendition file: {e}"))?;

    let size_bytes = tokio::fs::metadata(&abs)
        .await
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    Ok(StoredRendition {
        photo_id: candidate.photo_id.clone(),
        short_edge: rung.short_edge,
        width: rung.width,
        height: rung.height,
        is_source: 0,
        blob_id: None,
        file_path: Some(rel),
        codec: Some("h264".into()),
        bitrate: None,
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    //! These drive real FFmpeg through the real pass, because the two defects
    //! they guard are both invisible to a mocked encode: one is the *shape* of
    //! the bytes ffmpeg produced, and the other is which row the DB ends up
    //! holding after a probe that a mock would have to fake.
    //!
    //! Skipped when FFmpeg is unavailable so minimal CI images stay green —
    //! same convention as the #46 probe tests in `ingest.rs`.
    use super::*;
    use crate::conversion::{detect_parallelism, plan_parallelism};
    use crate::transcode::renditions::list_renditions;
    use std::str::FromStr;

    /// One encode at a time, one thread each — the budget the pure-logic and
    /// single-candidate tests use so they assert their own subject and not the
    /// host's core count.
    const SERIAL: SweepBudget = SweepBudget {
        lane: 1,
        threads: 1,
    };

    async fn test_pool() -> sqlx::SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at) \
             VALUES ('u1', 'u1', 'x', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    /// A scratch storage root that cleans itself up.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "sp_ladder_{}_{tag}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Encode a real clip of an exact size. `None` when FFmpeg is unavailable.
    fn make_video(root: &Path, rel: &str, size: &str) -> Option<PathBuf> {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc2=duration=1:size={size}:rate=10"),
                "-c:v",
                "libx264",
                "-profile:v",
                "high",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if ok
            && std::fs::metadata(&path)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        {
            Some(path)
        } else {
            None
        }
    }

    /// Encode a clip a browser cannot decode natively: MPEG-4 Part 2 in an `.mp4`
    /// container — exactly the 10-file class the #46 backfill exists for. `mpeg4`
    /// is built into ffmpeg (no libx265 dependency), so this is reliable on the
    /// same minimal images `make_video` runs on. `None` when ffmpeg is missing.
    fn make_nonnative_video(root: &Path, rel: &str, size: &str) -> Option<PathBuf> {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc2=duration=1:size={size}:rate=10"),
                "-c:v",
                "mpeg4",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if ok
            && std::fs::metadata(&path)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        {
            Some(path)
        } else {
            None
        }
    }

    /// Register a video. `width`/`height` are what the DB *claims*, which the
    /// pass must never use for geometry.
    async fn insert_video(pool: &sqlx::SqlitePool, id: &str, rel: &str, width: i64, height: i64) {
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, created_at) \
             VALUES (?, 'u1', ?, ?, 'video/mp4', 'video', 0, ?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("{id}.mp4"))
        .bind(rel)
        .bind(width)
        .bind(height)
        .execute(pool)
        .await
        .unwrap();
    }

    /// **The transposition test.** `photos.width`/`height` are transposed
    /// against ffprobe for a large part of the live library — the census counted
    /// 126 × `3840x2160` where the DB holds 78 of them as `2160x3840`.
    ///
    /// A pass that plans from the stored pair computes
    /// `rung_dimensions(2560, 1440, 1080)` = `1920x1080` for this portrait
    /// source and hands `scale=1920:1080` to ffmpeg, which squashes a portrait
    /// frame into a landscape box. The output is not merely mis-recorded — it is
    /// visibly distorted, and nothing downstream can recover it.
    ///
    /// So this asserts the dimensions of the file ffmpeg actually produced, not
    /// the row we wrote about it. Those are the two things that can disagree.
    #[tokio::test]
    async fn a_transposed_row_does_not_squash_a_portrait_rendition() {
        let root = TempRoot::new("transposed");
        // A real portrait 1440x2560 source: short edge 1440 > 1080, so a rung
        // is owed and it must come out 1080x1920.
        let Some(_) = make_video(&root.0, "videos/portrait.mp4", "1440x2560") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        // Transposed exactly as the live DB holds it.
        insert_video(&pool, "p1", "videos/portrait.mp4", 2560, 1440).await;

        let candidates = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        assert_eq!(
            candidates.len(),
            1,
            "the video must be selected as a candidate"
        );

        let verdict = generate_one(&pool, &root.0, None, &candidates[0], 1)
            .await
            .expect("rung generation must succeed");
        assert_eq!(verdict, Verdict::Produced { short_edge: 1080 });

        let rows = list_renditions(&pool, "p1").await.unwrap();
        let rung = rows
            .iter()
            .find(|r| !r.is_source())
            .expect("a 1080p rung must be recorded");
        assert_eq!(
            (rung.width, rung.height),
            (1080, 1920),
            "the rung must stay portrait; taking geometry from photos.width/height \
             would have produced 1920x1080"
        );

        // What ffmpeg actually wrote — the assertion the recorded row cannot make.
        let produced = root.0.join(rung.file_path.as_ref().unwrap());
        let info = probe::probe_video_stream(&produced).await.unwrap();
        assert_eq!(
            (info.width, info.height),
            (1080, 1920),
            "the encoded file itself must be portrait 1080x1920, not a squashed frame"
        );
    }

    /// The source rung must accompany the downscale, pointing at the bytes the
    /// photo already owns. A picker offering only 1080p on a 1440p video has
    /// silently taken away the quality the user had.
    #[tokio::test]
    async fn the_source_rung_is_offered_alongside_the_downscale() {
        let root = TempRoot::new("source_rung");
        let Some(_) = make_video(&root.0, "videos/big.mp4", "1440x2560") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        insert_video(&pool, "p1", "videos/big.mp4", 2560, 1440).await;

        let candidates = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        generate_one(&pool, &root.0, None, &candidates[0], 1)
            .await
            .unwrap();

        let rows = list_renditions(&pool, "p1").await.unwrap();
        assert_eq!(
            rows.len(),
            2,
            "picker must offer source + 1080p, got {rows:?}"
        );
        // Highest first — the order a "default to highest" client reads.
        assert!(rows[0].is_source(), "the source rung must sort first");
        assert_eq!(rows[0].short_edge, 1440);
        assert_eq!(
            rows[0].file_path.as_deref(),
            Some("videos/big.mp4"),
            "the source rung must point at the photo's existing bytes, not a copy"
        );
        assert_eq!(rows[1].short_edge, 1080);
    }

    /// **The terminal-verdict test.** 58 live videos have no recorded geometry,
    /// so the prefilter selects them blind and lets the probe decide. Most need
    /// no rung — and without `not_needed` (037) that answer has nowhere to go:
    /// the row keeps both locators NULL, the candidate query reads it as "still
    /// owed", and the file is re-probed on every sweep until the attempt cap
    /// retires it with a warning claiming it will never get a picker.
    ///
    /// Verified RED by dropping the `r.not_needed = 1` arm from the candidate
    /// query: the second selection returns the photo again.
    #[tokio::test]
    async fn a_video_below_the_tier_leaves_the_candidate_set_permanently() {
        let root = TempRoot::new("not_needed");
        let Some(_) = make_video(&root.0, "videos/small.mp4", "320x240") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        // Geometry unknown — the live shape that forces a blind selection.
        insert_video(&pool, "p1", "videos/small.mp4", 0, 0).await;

        let candidates = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        assert_eq!(
            candidates.len(),
            1,
            "a row with no recorded geometry must be selected and resolved by a probe"
        );

        let verdict = generate_one(&pool, &root.0, None, &candidates[0], 1)
            .await
            .unwrap();
        assert_eq!(verdict, Verdict::NotNeeded);

        let again = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        assert!(
            again.is_empty(),
            "a probed-and-not-owed video must never be selected again, got {again:?}"
        );

        // And the verdict must not masquerade as a playable rendition.
        assert!(
            list_renditions(&pool, "p1").await.unwrap().is_empty(),
            "a not-needed verdict must not surface in the picker"
        );

        // No attempt was spent: nothing was encoded, so a file later replaced
        // with a genuine 4K source starts with its full retry budget.
        let attempts: i64 =
            sqlx::query_scalar("SELECT attempt_count FROM video_renditions WHERE photo_id = 'p1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(attempts, 0);
    }

    /// A photo still in the encryption backlog must be left alone. Both clients
    /// play from blobs, so a `file_path` rendition produced for a row that is
    /// about to be encrypted is bytes nothing can play — and the encode that
    /// made them would have to be repeated afterwards anyway.
    ///
    /// 2,494 live rows are in this state, so this is the normal case on a box
    /// mid-migration, not an edge one.
    #[tokio::test]
    async fn a_photo_awaiting_encryption_is_deferred_not_encoded() {
        let root = TempRoot::new("backlog");
        let Some(_) = make_video(&root.0, "videos/big.mp4", "1440x2560") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        // No encrypted_blob_id, but the install has a key → encrypted mode.
        insert_video(&pool, "p1", "videos/big.mp4", 2560, 1440).await;

        let candidates = rung_queue::find_rung_candidates(&pool, 10).await.unwrap();
        let verdict = generate_one(&pool, &root.0, Some(&[7u8; 32]), &candidates[0], 1)
            .await
            .unwrap();

        assert!(
            matches!(verdict, Verdict::Skipped(ref r) if r.contains("awaiting encryption")),
            "expected a deferral, got {verdict:?}"
        );
        // Nothing recorded: no claim, no attempt charged, no rendition. The row
        // must come back as a candidate once the migration gives it a blob.
        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM video_renditions WHERE photo_id = 'p1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows, 0,
            "a deferral must not spend an attempt or claim a rung"
        );
    }

    /// A produced rung removes its photo from the queue — the property that
    /// makes the candidate set self-limiting on success, and therefore the
    /// reason a sweep is safe to run on every autoscan.
    #[tokio::test]
    async fn a_produced_rung_is_not_re_encoded_on_the_next_sweep() {
        let root = TempRoot::new("idempotent");
        let Some(_) = make_video(&root.0, "videos/big.mp4", "1440x2560") else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        insert_video(&pool, "p1", "videos/big.mp4", 2560, 1440).await;

        let first = run_sweep_with_budget(&pool, &root.0, "test-secret", SERIAL).await;
        assert_eq!(
            first.produced, 1,
            "first sweep must produce the rung: {first:?}"
        );

        let second = run_sweep_with_budget(&pool, &root.0, "test-secret", SERIAL).await;
        assert_eq!(
            second,
            SweepOutcome::default(),
            "a second sweep must find no work at all, got {second:?}"
        );
    }

    // ── #49 cost control: the sweep's concurrency budget ─────────────────────

    /// The budget's two halves must come from **one** plan. They were two
    /// derivations before 2026-08-04: the driver ran serially while
    /// `transcode_to_rung` independently computed `-threads` as
    /// `usable / video_lane` — a share sized for `video_lane` concurrent
    /// encodes, of which exactly one ever ran.
    ///
    /// So this asserts the arithmetic that made the old pairing wrong: on a
    /// many-core host the lane is wide and the per-encode share is a *fraction*
    /// of the budget, and only running `lane` of them spends the whole of it.
    #[test]
    fn the_sweep_budget_is_one_plans_two_halves() {
        let plan = plan_parallelism(128, false);
        let budget = plan_sweep_budget(plan);

        assert_eq!(budget.lane, plan.video_lane);
        assert_eq!(budget.threads, plan.video_threads);
        assert!(
            budget.lane > 1,
            "a 128-thread host must run rungs in parallel, got lane={}",
            budget.lane
        );
        // The defect, stated as arithmetic: one encode at this thread budget
        // uses a small fraction of the cores the plan reserved for the lane.
        assert!(
            budget.threads * 4 < plan.fast_lane,
            "a serial sweep at {} threads would leave most of the {} usable \
             cores idle — that is what the lane fixes",
            budget.threads,
            plan.fast_lane
        );
    }

    /// `video_lane` is 1 below ~24 threads, so the lane is a **no-op** on
    /// ordinary hardware. This is the claim that makes the change safe to ship
    /// without measuring every deployment: nothing gets wider unless there are
    /// cores to make it wider with.
    #[test]
    fn the_sweep_stays_serial_on_an_ordinary_host() {
        for cores in [1usize, 2, 4, 8, 16] {
            let budget = plan_sweep_budget(plan_parallelism(cores, false));
            assert_eq!(
                budget.lane, 1,
                "{cores} cores must stay serial, got lane={}",
                budget.lane
            );
        }
    }

    /// The GPU path is capped at the hardware session count, not at the core
    /// count — the reason the ladder reuses the *video* lane rather than the
    /// fast one. Over-subscribing NVENC/QSV sessions fails or silently
    /// serialises; a rendition sweep at `fast_lane` width would do exactly that.
    #[test]
    fn the_sweep_respects_the_gpu_session_cap() {
        let budget = plan_sweep_budget(plan_parallelism(128, true));
        let plan = plan_parallelism(128, true);
        assert_eq!(budget.lane, plan.video_lane);
        assert!(
            budget.lane < plan.fast_lane,
            "the GPU lane must stay far below the image lane, got {} vs {}",
            budget.lane,
            plan.fast_lane
        );
    }

    /// `SIMPLE_PHOTOS_CONVERSION_JOBS` must reach the ladder.
    ///
    /// It did not: `transcode_to_rung` planned its own threads from
    /// `plan_parallelism(num_cpus::get(), ..)`, which never reads the variable.
    /// An operator pinning the budget to keep a box calm got a quiet conversion
    /// pass and a ladder still sized for the whole machine.
    ///
    /// Asserted through the *planner identity* rather than by setting the env
    /// var: `cargo test` runs these on shared threads, and a process-global
    /// mutation would race every other test in the binary. `detect_parallelism`
    /// is the only entry point that consults the override, so "the sweep plans
    /// with `detect_parallelism`" is the property, and it is the one that was
    /// false.
    #[test]
    fn the_sweep_budget_comes_from_the_overridable_planner() {
        let gpu = crate::conversion::active_hwaccel()
            .map(|h| h.is_gpu())
            .unwrap_or(false);
        assert_eq!(
            plan_sweep_budget(detect_parallelism(gpu)),
            plan_sweep_budget(detect_parallelism(gpu)),
            "the planner must be deterministic within a process"
        );
        // With no override set the two planners must agree — that equality is
        // what makes swapping `plan_parallelism` for `detect_parallelism` a
        // pure widening rather than a behaviour change on an unconfigured box.
        // Skipped when an operator has actually set the variable, because then
        // they are *supposed* to disagree and asserting otherwise would be a
        // test that passes only on a machine nobody configured.
        if std::env::var("SIMPLE_PHOTOS_CONVERSION_JOBS").is_err() {
            assert_eq!(
                plan_sweep_budget(detect_parallelism(gpu)),
                plan_sweep_budget(plan_parallelism(num_cpus::get(), gpu)),
                "with no override set, detect_parallelism must equal plan_parallelism"
            );
        }
    }

    /// Encryption outranks conversion outranks the ladder, and the ladder never
    /// runs *beside* a conversion pass.
    ///
    /// The conversion arm is the new one. Until 2026-08-04 the ordering was
    /// arranged only by the three autoscan call sites awaiting
    /// `run_conversion_pass` first — but `upload.rs` and `scan.rs` kick that
    /// pass with no such sequencing, so an upload landing mid-sweep put two
    /// `video_lane`-wide lanes on one box. On the GPU path that is literally
    /// double the hardware session cap.
    #[test]
    fn the_sweep_yields_to_encryption_and_to_conversion() {
        assert_eq!(should_defer_sweep(false, false), None, "idle box: proceed");
        assert_eq!(
            should_defer_sweep(false, true),
            Some("conversion pass running"),
            "a rung must never compete with the pass that makes videos playable"
        );
        assert_eq!(
            should_defer_sweep(true, false),
            Some("encryption migration active")
        );
        // Encryption is reported first when both hold: it is the outer
        // precondition, and the log should name the thing that will clear last.
        assert_eq!(
            should_defer_sweep(true, true),
            Some("encryption migration active")
        );
    }

    /// **The lane, end to end.** Three oversized videos through one sweep at
    /// `lane = 3` must overlap — and must still produce exactly three correct
    /// renditions, because a wider lane that double-encodes or mis-tallies is
    /// worse than a serial one.
    ///
    /// Verified RED by restoring the serial `for` loop in `process_phase`:
    /// `peak_concurrency` comes back 1. The counters stay green in that run,
    /// which is the point — the peak is the only assertion that can tell a
    /// parallel driver from a serial one.
    #[tokio::test]
    async fn a_wide_lane_encodes_several_rungs_at_once() {
        let root = TempRoot::new("lane");
        for n in 0..3 {
            if make_video(&root.0, &format!("videos/v{n}.mp4"), "1440x2560").is_none() {
                eprintln!("ffmpeg/libx264 unavailable — skipping");
                return;
            }
        }

        let pool = test_pool().await;
        for n in 0..3 {
            insert_video(
                &pool,
                &format!("p{n}"),
                &format!("videos/v{n}.mp4"),
                2560,
                1440,
            )
            .await;
        }

        let wide = SweepBudget {
            lane: 3,
            threads: 1,
        };
        let outcome = run_sweep_with_budget(&pool, &root.0, "test-secret", wide).await;

        assert_eq!(
            (outcome.examined, outcome.produced, outcome.failed),
            (3, 3, 0),
            "every candidate must produce its rung: {outcome:?}"
        );
        assert!(
            outcome.peak_concurrency > 1,
            "the lane must actually overlap encodes — a serial driver reports 1, \
             got {}",
            outcome.peak_concurrency
        );

        // Correctness under concurrency: one rung + one source row each, and
        // exactly one attempt charged. A lane that let two workers claim the
        // same photo would show up here as a second attempt.
        for n in 0..3 {
            let id = format!("p{n}");
            let rows = list_renditions(&pool, &id).await.unwrap();
            assert_eq!(rows.len(), 2, "{id}: source + 1080p expected, got {rows:?}");
            assert_eq!(rows[1].short_edge, 1080);
            let attempts: i64 = sqlx::query_scalar(
                "SELECT attempt_count FROM video_renditions WHERE photo_id = ? AND is_source = 0",
            )
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(attempts, 1, "{id}: exactly one encode attempt");
        }

        // And the sweep is still self-limiting: nothing is owed afterwards.
        let second = run_sweep_with_budget(&pool, &root.0, "test-secret", wide).await;
        assert_eq!(
            second,
            SweepOutcome::default(),
            "a wide sweep must retire its candidates like a serial one, got {second:?}"
        );
    }

    /// A serial budget must stay serial — the property that makes the "no-op on
    /// ordinary hardware" claim observable rather than merely argued.
    #[tokio::test]
    async fn a_serial_budget_never_overlaps() {
        let root = TempRoot::new("serial");
        for n in 0..3 {
            if make_video(&root.0, &format!("videos/v{n}.mp4"), "1440x2560").is_none() {
                eprintln!("ffmpeg/libx264 unavailable — skipping");
                return;
            }
        }

        let pool = test_pool().await;
        for n in 0..3 {
            insert_video(
                &pool,
                &format!("p{n}"),
                &format!("videos/v{n}.mp4"),
                2560,
                1440,
            )
            .await;
        }

        let outcome = run_sweep_with_budget(&pool, &root.0, "test-secret", SERIAL).await;
        assert_eq!(outcome.produced, 3);
        assert_eq!(
            outcome.peak_concurrency, 1,
            "lane = 1 must run exactly one encode at a time, got {}",
            outcome.peak_concurrency
        );
    }

    /// The #46 backfill, end to end. A registered small non-native video (MPEG-4
    /// Part 2, which no browser decodes) is re-encoded to a source-resolution
    /// H.264 rendition through the SAME pass as the resolution ladder. The ladder
    /// never selects it — it is far below the tier — so this exercises the
    /// backfill phase specifically, and the whole reason the file was invisible
    /// before: `existing_set` skipped it at ingest and the ladder rule excluded it.
    #[tokio::test]
    async fn the_backfill_reencodes_a_small_non_native_video_to_h264() {
        let root = TempRoot::new("codec_backfill");
        let Some(_) = make_nonnative_video(&root.0, "videos/sd.mp4", "320x240") else {
            eprintln!("ffmpeg/mpeg4 unavailable — skipping");
            return;
        };

        let pool = test_pool().await;
        insert_video(&pool, "p1", "videos/sd.mp4", 320, 240).await;

        // The two candidate sets prove the routing: the ladder does not see it,
        // the backfill does.
        assert!(
            rung_queue::find_rung_candidates(&pool, 10)
                .await
                .unwrap()
                .is_empty(),
            "a 320x240 source is far below the tier — the resolution ladder must ignore it"
        );
        assert_eq!(
            rung_queue::find_codec_backfill_candidates(&pool, 10)
                .await
                .unwrap()
                .len(),
            1,
            "the codec backfill must pick up the registered small container"
        );

        let outcome = run_sweep_with_budget(&pool, &root.0, "test-secret", SERIAL).await;
        assert_eq!(
            (outcome.backfill_examined, outcome.backfill_produced),
            (1, 1),
            "the sweep must examine and fix it as backfill work, got {outcome:?}"
        );

        // Exactly one playable rung, at the source resolution, and NOT the source
        // (the unplayable original must never be offered as "Original").
        let rungs = list_renditions(&pool, "p1").await.unwrap();
        assert_eq!(
            rungs.len(),
            1,
            "one playable rung, no unplayable original: {rungs:?}"
        );
        assert!(
            !rungs[0].is_source(),
            "the mpeg4 source itself is not offerable"
        );
        assert_eq!(rungs[0].short_edge, 240);

        // What ffmpeg actually wrote is H.264 — the codec fix, not merely a row.
        let produced = root.0.join(rungs[0].file_path.as_ref().unwrap());
        let info = probe::probe_video_stream(&produced).await.unwrap();
        assert_eq!(
            info.codec, "h264",
            "the re-encode must be browser-native H.264"
        );
        assert!(
            probe::is_browser_native(&info),
            "the produced rendition must actually be playable, got {info:?}"
        );
        assert_eq!(
            (info.width, info.height),
            (320, 240),
            "codec-only: resolution is unchanged"
        );

        // One-shot: the produced is_source=0 rung retires it from the backfill set.
        let second = run_sweep_with_budget(&pool, &root.0, "test-secret", SERIAL).await;
        assert_eq!(
            second,
            SweepOutcome::default(),
            "the backfill must examine each file exactly once, got {second:?}"
        );
    }
}
