//! Conversion ingest engine — runs AFTER native file import and encryption
//! are complete. Discovers non-native media files on disk, converts them to
//! browser-compatible formats via FFmpeg, registers the converted files in
//! the database, and triggers encryption for the newly registered files.
//!
//! This module enforces strict sequencing:
//!   1. Native files are imported and encrypted (handled by scan + server_migrate)
//!   2. THIS module then discovers convertible files
//!   3. Converts them to a `.converted/` staging folder
//!   4. Registers the converted results in the DB
//!   5. Triggers encryption for the newly converted files
//!
//! This prevents the race condition where conversion and encryption run
//! simultaneously on different files.

use std::path::PathBuf;
use std::sync::OnceLock;

use futures_util::stream::{self, StreamExt};
use futures_util::TryStreamExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::conversion;
use crate::photos::metadata::extract_media_metadata_async;
use crate::photos::thumbnail::generate_thumbnail_file;
use crate::photos::utils::{compute_photo_hash_streaming, normalize_iso_timestamp, utc_now_iso};

/// Serializes conversion passes so only one runs at a time.
/// Prevents concurrent passes from racing on the global progress counters.
fn conversion_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Set when a conversion trigger arrives while a pass is already running. The
/// active pass checks this before releasing the lock and performs one more
/// sweep if set, so files that landed mid-walk are never stranded until the
/// next autoscan tick.
static CONVERSION_RERUN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Pure decision for [`ingest_pipeline_busy`], split out so the phase-ordering
/// invariant can be unit-tested without a live database or running migration.
///
/// The pipeline is "busy" (and downstream AI / geo work must wait) while:
///   * an encryption migration is running, OR
///   * a conversion pass is converting / encrypting non-native files, OR
///   * native files are registered but not yet encrypted — *but only when an
///     encryption key actually exists*.  Without that guard, a no-key install
///     (no client has ever logged in to provide the key) would report "busy"
///     forever and permanently starve the AI / geo processors.
fn pipeline_busy_decision(
    migration_running: bool,
    conversion_running: bool,
    has_key: bool,
    pending_unencrypted: bool,
) -> bool {
    migration_running || conversion_running || (has_key && pending_unencrypted)
}

/// Returns `true` while the import → encrypt → convert pipeline still has work
/// in flight.  The background AI and geo processors poll this and defer their
/// batches until it clears, implementing the intended phase order:
/// **import + encrypt everything → convert what needs it → then AI + geo**.
///
/// Without this gate the three pipelines run concurrently and contend for the
/// CPU and SQLite's single writer lock, which is the "jumping between encrypt,
/// AI and geo one photo at a time" stall seen on large imports.
pub async fn ingest_pipeline_busy(pool: &sqlx::SqlitePool) -> bool {
    // Cheap, lock-free signals first.
    if crate::photos::server_migrate::migration_active().await {
        return true;
    }
    if crate::conversion::progress_snapshot().0 {
        return true;
    }

    // Only treat "files awaiting encryption" as busy when a key exists to
    // encrypt them — otherwise nothing will ever clear it (see the guard note
    // on `pipeline_busy_decision`).
    let has_key: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM server_settings WHERE key = 'encryption_key_wrapped'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false);

    let pending_unencrypted: bool = if has_key {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM photos WHERE encrypted_blob_id IS NULL AND encryption_deferred = 0)",
        )
            .fetch_one(pool)
            .await
            .unwrap_or(false)
    } else {
        false
    };

    pipeline_busy_decision(false, false, has_key, pending_unencrypted)
}

/// Run a conversion pass: discover non-native files, convert, register, encrypt.
///
/// Called AFTER `auto_migrate_after_scan()` completes so that native files
/// are fully encrypted before we start FFmpeg conversions.
pub async fn run_conversion_pass(
    pool: sqlx::SqlitePool,
    storage_root: PathBuf,
    jwt_secret: String,
) {
    use std::sync::atomic::Ordering;

    // Only one conversion pass runs at a time. If one is already in flight, set
    // a rerun flag and return immediately instead of spawning a task that
    // blocks on `lock().await`. Autoscan (every ~5 min) and every deferred
    // upload spawn a fresh call here; on a large library those blocked tasks
    // would otherwise accumulate without bound and exhaust memory. The running
    // pass honors the flag with one extra sweep so nothing is missed.
    let _guard = match conversion_lock().try_lock() {
        Ok(g) => g,
        Err(_) => {
            CONVERSION_RERUN.store(true, Ordering::SeqCst);
            tracing::debug!("[INGEST] Conversion pass already running; queued a follow-up sweep");
            return;
        }
    };

    loop {
        CONVERSION_RERUN.store(false, Ordering::SeqCst);
        run_conversion_pass_inner(pool.clone(), storage_root.clone(), jwt_secret.clone()).await;
        if !CONVERSION_RERUN.swap(false, Ordering::SeqCst) {
            break;
        }
        tracing::info!(
            "[INGEST] Re-running conversion pass to pick up files that arrived during the previous pass"
        );
    }
}

/// A non-native file discovered on disk that needs converting to a browser
/// format before it can be registered.
struct ConvertCandidate {
    abs_path: PathBuf,
    rel_path: String,
    name: String,
    target: conversion::ConversionTarget,
    size: i64,
    modified: Option<String>,
    /// Set when this file is a *corrupt-bitstream salvage* rather than an ordinary
    /// conversion (#46). Carried so the `MediaConvert` audit row can record that
    /// the output is shorter than the source — the loss must be surfaced, not
    /// silent. `None` for every ordinary conversion.
    salvage: Option<crate::transcode::probe::DecodeHealth>,
}

/// Shared, cheaply-cloneable context for the concurrent per-file conversion
/// workers. Everything here is read-only for the duration of a pass except the
/// database (which serialises its own writes), so it can be borrowed across all
/// the in-flight `for_each_concurrent` tasks.
struct ConvCtx {
    pool: sqlx::SqlitePool,
    storage_root: PathBuf,
    admin_id: String,
    conv_dir: PathBuf,
    pano_sensitivity: crate::photos::metadata::PanoSensitivity,
}

/// Convert one candidate to a browser-native format and register it (or, on a
/// conversion failure, register the original to avoid data loss). Returns `true`
/// when a new row was registered so the caller can tally the batch.
///
/// This is the body run concurrently across the CPU cores. The heavy step is the
/// `convert_file` transcode; the DB writes use `INSERT OR IGNORE` + hash dedup so
/// concurrent workers (and concurrent scans) can't create duplicates.
/// `video_threads` bounds each CPU video encode's thread count so a lane running
/// several encodes at once doesn't oversubscribe the cores.
async fn process_candidate(
    candidate: &ConvertCandidate,
    ctx: &ConvCtx,
    video_threads: Option<usize>,
) -> bool {
    let ConvCtx {
        pool,
        storage_root,
        admin_id,
        conv_dir,
        pano_sensitivity,
    } = ctx;
    let pano_sensitivity = *pano_sensitivity;

    let conv_id = Uuid::new_v4();
    let conv_filename = format!("{}.{}", conv_id, candidate.target.extension);
    let conv_abs = conv_dir.join(&conv_filename);
    let conv_rel = format!(".converted/{conv_filename}");

    // Log BEFORE the transcode with file + category so that, if a single
    // file hangs (the Windows "conversion stalls" report, #10), the last
    // "converting" line names the culprit — the matching "converted in"
    // line below never appears for a stuck file.
    let file_start = std::time::Instant::now();
    tracing::info!(
        file = %candidate.name,
        category = ?candidate.target.category,
        size_bytes = candidate.size,
        "[INGEST] converting"
    );

    // Heartbeat: a long transcode emits no `progress_tick` until it returns
    // (up to the GPU + CPU attempt budgets), so pulse liveness every ~20s
    // WHILE this file converts — otherwise the frontend's short-fuse "looks
    // stuck" banner fires mid-encode on a large/slow video. Bounded to ~20 min
    // so a file that somehow never returns (e.g. an unkillable ffmpeg on a dead
    // network mount) stops heartbeating and the 2h stuck-job watchdog can still
    // force-recover the pipeline.
    // #40: charge this attempt BEFORE the encode, not in the failure handler.
    // The files that most need retiring are the ones that never reach a failure
    // handler at all — an ffmpeg that OOMs, a hard kill, a pass cancelled by the
    // stuck-job watchdog. Charging afterwards means those specific files are
    // never counted and loop forever, which is the failure mode this cap exists
    // to end. Same reasoning as the rendition attempt cap in migration 036.
    //
    // The row is written even though the file may be about to succeed; the
    // success path clears it below.
    let attempt = crate::photos::register::charge_conversion_attempt(
        pool,
        admin_id,
        &candidate.rel_path,
        candidate.size,
        candidate.modified.as_deref(),
        &utc_now_iso(),
    )
    .await;

    let heartbeat = tokio::spawn(async {
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            conversion::heartbeat();
        }
    });
    // Starts the wall-clock this category's throughput is measured against
    // (#40). Only the first file of a category does anything, so the measured
    // rate covers the whole lane rather than one encode at a time — which is
    // what makes it a throughput the *remaining* queue will actually see.
    conversion::eta_start(candidate.target.category);
    let convert_result = conversion::convert_file(
        &candidate.abs_path,
        &conv_abs,
        &candidate.target,
        video_threads,
    )
    .await;
    heartbeat.abort();

    match convert_result {
        Ok(()) => {
            conversion::progress_tick();
            conversion::eta_complete(candidate.target.category, candidate.size);
            tracing::info!(
                file = %candidate.name,
                category = ?candidate.target.category,
                elapsed_ms = file_start.elapsed().as_millis(),
                "[INGEST] converted in"
            );
            let new_name = {
                let stem = std::path::Path::new(&candidate.name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("converted");
                format!("{}.{}", stem, candidate.target.extension)
            };
            tracing::info!(
                original = %candidate.name,
                converted = %new_name,
                "[INGEST] Converted file to browser-native format"
            );

            // Audit the conversion so it's visible in the Server Logs tab.
            // Streams live via the globally-registered broadcast sender.
            let mut convert_details = serde_json::json!({
                "filename": candidate.name,
                "converted": new_name,
                "category": conversion::media_type_str(candidate.target.category),
                "size_bytes": candidate.size,
                "elapsed_ms": file_start.elapsed().as_millis() as u64,
            });
            if let (Some(health), Some(obj)) =
                (&candidate.salvage, convert_details.as_object_mut())
            {
                // #46: this was a corrupt-bitstream salvage. ffmpeg keeps the
                // frames it can read and drops the rest, so the output is SHORTER
                // than the source — record the loss instead of presenting a
                // silently-truncated video as a clean conversion.
                obj.insert("salvage".into(), serde_json::Value::Bool(true));
                obj.insert("decode_errors".into(), serde_json::json!(health.error_count));
                obj.insert("first_error".into(), serde_json::json!(health.first_error));
                obj.insert(
                    "note".into(),
                    serde_json::Value::String(
                        "corrupt source salvaged; output is shorter than the original".into(),
                    ),
                );
            }
            crate::audit::log_background(
                pool,
                crate::audit::AuditEvent::MediaConvert,
                Some(convert_details),
            );

            // ── Register the converted file in the DB ────
            let photo_id = Uuid::new_v4().to_string();
            let now = utc_now_iso();
            let work_mime = candidate.target.mime_type;
            let work_media_type = conversion::media_type_str(candidate.target.category);
            let thumb_ext = if work_mime == "image/gif" {
                "gif"
            } else {
                "jpg"
            };
            let thumb_rel = format!(".thumbnails/{photo_id}.thumb.{thumb_ext}");

            // Extract metadata from the ORIGINAL file first — it has the real
            // EXIF DateTimeOriginal, GPS, and camera data.  Conversion
            // (FFmpeg/ImageMagick) typically strips EXIF from the output,
            // so reading the converted file would lose the original dates.
            let (_, _, orig_cam, orig_lat, orig_lon, orig_taken, orig_taken_offset) =
                extract_media_metadata_async(candidate.abs_path.clone()).await;

            // Extract dimensions from the converted file (the output format
            // may have different dimensions due to SAR correction, etc.).
            let (img_w, img_h, conv_cam, conv_lat, conv_lon, conv_taken, _) =
                extract_media_metadata_async(conv_abs.clone()).await;

            // ── Subtype detection from the ORIGINAL file ─────────────
            // Conversion strips XMP, so an iPhone/Samsung HEIC panorama,
            // 360° sphere, burst frame, or motion photo would lose its
            // nature forever if we only ever looked at the converted
            // JPEG.  Scan the original's prefix instead.
            let mut subtype_info = if candidate.target.category == conversion::MediaCategory::Image
            {
                let prefix = crate::photos::metadata::read_file_prefix(
                    &candidate.abs_path,
                    crate::photos::metadata::XMP_SCAN_PREFIX_BYTES,
                )
                .await;
                crate::photos::metadata::extract_xmp_subtype(&prefix)
            } else {
                Default::default()
            };
            if candidate.target.category == conversion::MediaCategory::Image {
                crate::photos::metadata::apply_aspect_subtype_fallback_with(
                    &mut subtype_info,
                    img_w,
                    img_h,
                    pano_sensitivity,
                );
            }

            // Prefer original file's metadata; fall back to converted, then mtime.
            let cam_model = orig_cam.or(conv_cam);
            let exif_lat = orig_lat.or(conv_lat);
            let exif_lon = orig_lon.or(conv_lon);
            let final_taken_at = orig_taken
                .map(|t| normalize_iso_timestamp(&t))
                .or(conv_taken.map(|t| normalize_iso_timestamp(&t)))
                .or(candidate.modified.clone());
            // Only the original carries EXIF; conversion strips the zone.
            let final_taken_offset = orig_taken_offset;

            let photo_hash = compute_photo_hash_streaming(&conv_abs).await;

            // Hash-based dedup: skip if an identical file was already registered
            // (catches re-conversion of the same source across concurrent scans).
            if let Some(ref hash) = photo_hash {
                let dup_exists: bool = sqlx::query_scalar(
                    "SELECT COUNT(*) > 0 FROM photos WHERE photo_hash = ? AND user_id = ?",
                )
                .bind(hash)
                .bind(admin_id)
                .fetch_one(pool)
                .await
                .unwrap_or(false);

                if dup_exists {
                    tracing::debug!(
                        hash = %hash,
                        file = %candidate.name,
                        "[INGEST] Duplicate hash detected, skipping"
                    );
                    // Clean up the converted file we just created
                    let _ = tokio::fs::remove_file(&conv_abs).await;
                    // #40: record the dead end so the next pass does not
                    // transcode this file again just to throw the output away.
                    //
                    // This is the most expensive loop in the issue. On a Google
                    // Takeout library the same bytes appear in the date folder
                    // AND in every album folder; the date-folder copy registers
                    // first, so every album copy lands here — after a full
                    // GPU-then-CPU transcode — on every single pass, forever.
                    // The native-file walk has skipped exactly this case since
                    // migration 031 (`record_scan_skip(.., "hash_duplicate")`);
                    // the conversion walk never learned to.
                    //
                    // Recorded as a TERMINAL `hash_duplicate`, not as a
                    // conversion attempt: nothing failed, and the answer is
                    // deterministic, so spending three transcodes to reach it
                    // three times would be its own bug. Carrying `photo_hash`
                    // also puts the row under 031's delete-triggers, so deleting
                    // the photo this deduped against re-admits the copy.
                    crate::photos::register::record_scan_skip_path(
                        pool,
                        admin_id,
                        &candidate.rel_path,
                        candidate.size,
                        candidate.modified.as_deref(),
                        crate::photos::scan_skip::REASON_HASH_DUPLICATE,
                        Some(hash),
                        &utc_now_iso(),
                    )
                    .await;
                    return false;
                }
            }

            let final_size = tokio::fs::metadata(&conv_abs)
                .await
                .map(|m| m.len() as i64)
                .unwrap_or(candidate.size);

            let source_path = Some(candidate.rel_path.clone());

            let insert_result = sqlx::query(
                "INSERT OR IGNORE INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                 size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, \
                 created_at, photo_hash, source_path, photo_subtype, burst_id, taken_at_offset) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&photo_id)
            .bind(admin_id)
            .bind(&new_name)
            .bind(&conv_rel)
            .bind(work_mime)
            .bind(work_media_type)
            .bind(final_size)
            .bind(img_w)
            .bind(img_h)
            .bind(&final_taken_at)
            .bind(exif_lat)
            .bind(exif_lon)
            .bind(&cam_model)
            .bind(&thumb_rel)
            .bind(&now)
            .bind(&photo_hash)
            .bind(&source_path)
            .bind(&subtype_info.photo_subtype)
            .bind(&subtype_info.burst_id)
            .bind(&final_taken_offset)
            .execute(pool)
            .await;

            match insert_result {
                Ok(result) if result.rows_affected() == 0 => {
                    tracing::debug!(
                        file = %conv_rel,
                        "[INGEST] Already registered (concurrent scan), skipping"
                    );
                    return false;
                }
                Err(e) => {
                    tracing::error!(
                        file = %conv_rel,
                        error = %e,
                        "[INGEST] Failed to register converted photo"
                    );
                    // The bytes converted fine; the DB write did not. This path
                    // leaves NO row behind, so the file is re-walked, re-
                    // converted and re-failed on every autoscan pass forever —
                    // the exact failure mode #40's attempt cap exists to retire,
                    // and it is invisible until it is audited.
                    crate::audit::log_background(
                        pool,
                        crate::audit::AuditEvent::ImportFailure,
                        Some(serde_json::json!({
                            "filename": candidate.name,
                            "converted": conv_rel,
                            "source_path": source_path,
                            "category": conversion::media_type_str(candidate.target.category),
                            "error": e.to_string(),
                            "elapsed_ms": file_start.elapsed().as_millis() as u64,
                        })),
                    );
                    return false;
                }
                Ok(_) => {}
            }

            // #40: the file converted and registered, so the attempt row has
            // done its job. Left behind it is normally invisible — the path is
            // in `existing_set` from now on — but it would resurface as a
            // spurious retirement if this photo were later deleted and the file
            // re-scanned, retiring a file that demonstrably converts fine.
            crate::photos::register::clear_scan_skip(pool, admin_id, &candidate.rel_path).await;

            // Motion photos keep their MP4 trailer in the ORIGINAL file
            // (conversion drops it) — extract and store it now so the
            // viewer's LIVE playback works for converted HEICs too.
            if subtype_info.photo_subtype.as_deref() == Some("motion") {
                let orig_bytes = tokio::fs::read(&candidate.abs_path)
                    .await
                    .unwrap_or_default();
                if !orig_bytes.is_empty() {
                    crate::photos::motion::extract_and_store_motion_video(
                        pool,
                        storage_root,
                        admin_id,
                        &photo_id,
                        &orig_bytes,
                        subtype_info.motion_video_offset,
                    )
                    .await;
                } else {
                    tracing::warn!(
                        file = %candidate.name,
                        "[INGEST] Motion photo original unreadable — video trailer not extracted"
                    );
                }
            }

            // Generate thumbnail for the converted file.
            let thumb_abs = storage_root.join(&thumb_rel);
            if generate_thumbnail_file(&conv_abs, &thumb_abs, work_mime, None).await {
                tracing::debug!(file = %conv_rel, "[INGEST] Generated thumbnail");
            } else {
                tracing::warn!(file = %conv_rel, "[INGEST] Failed to generate thumbnail");
                // Non-fatal — the photo is registered and downloadable — but it
                // renders as a placeholder for good, which looks like a client
                // bug from the outside. Audited so it is attributable to a file.
                crate::audit::log_background(
                    pool,
                    crate::audit::AuditEvent::ThumbnailFailure,
                    Some(serde_json::json!({
                        "filename": candidate.name,
                        "converted": conv_rel,
                        "mime_type": work_mime,
                        "category": conversion::media_type_str(candidate.target.category),
                    })),
                );
            }

            true
        }
        Err(e) => {
            conversion::progress_tick();
            // Charge the weight on the failure path too (#40). A failed
            // transcode still burned the wall-clock the throughput is measured
            // against, so skipping it would make the rate climb silently as
            // failures accumulate — and leave this file's weight outstanding
            // forever, so the ETA never drains to zero.
            conversion::eta_complete(candidate.target.category, candidate.size);
            // Conversion failed (unsupported codec, GPU failure with CPU
            // fallback disabled, a transcode crash, …). Do NOT silently drop
            // the file — that loses data and makes the library smaller than
            // the source on disk (issue #1: "reported size lower than actual;
            // possible missing files"). Register the ORIGINAL in place so it
            // is counted, encrypted/backed up in step 5, and downloadable. It
            // may not render natively, but the bytes are preserved.
            tracing::warn!(
                file = %candidate.name,
                category = ?candidate.target.category,
                elapsed_ms = file_start.elapsed().as_millis(),
                error = %e,
                "[INGEST] Conversion failed — registering ORIGINAL to avoid data loss"
            );
            // The event this whole issue is about. `tracing::warn!` above goes
            // to the process log; the Server Logs tab reads the `audit` table
            // and would show this file as simply absent.
            //
            // Emitted on EVERY attempt, not just the last — #40's 3-strike cap
            // needs the attempt history to be visible, and a file retired after
            // three silent failures is indistinguishable from one that was never
            // seen. `size_bytes` and `elapsed_ms` are included because they are
            // what make a pattern legible across many failures (a whole category
            // failing instantly is a different problem from one 8K file timing
            // out).
            crate::audit::log_background(
                pool,
                crate::audit::AuditEvent::MediaConvertFailure,
                Some(serde_json::json!({
                    "filename": candidate.name,
                    "source_path": candidate.rel_path,
                    "category": conversion::media_type_str(candidate.target.category),
                    "size_bytes": candidate.size,
                    "error": e.to_string(),
                    "elapsed_ms": file_start.elapsed().as_millis() as u64,
                })),
            );
            // #40: announce retirement when this attempt was the last one. A
            // file dropped after three silent failures is otherwise
            // indistinguishable from one that was never scanned — which is the
            // complaint in #45 restated one level up. Emitted in ADDITION to the
            // per-attempt failure above, because "this failed" and "we have
            // stopped trying" are different facts and only the second one
            // explains a file that never appears again.
            if attempt.map(crate::photos::scan_skip::is_retired) == Some(true) {
                tracing::warn!(
                    file = %candidate.name,
                    attempts = crate::photos::scan_skip::CONVERSION_MAX_ATTEMPTS,
                    "[INGEST] Retiring file after repeated conversion failures — \
                     it will not be retried until it changes on disk"
                );
                crate::audit::log_background(
                    pool,
                    crate::audit::AuditEvent::ConversionRetired,
                    Some(serde_json::json!({
                        "filename": candidate.name,
                        "source_path": candidate.rel_path,
                        "category": conversion::media_type_str(candidate.target.category),
                        "attempts": crate::photos::scan_skip::CONVERSION_MAX_ATTEMPTS,
                        "size_bytes": candidate.size,
                        "error": e.to_string(),
                        // Told to the user rather than left implicit: this is
                        // the only way back, and it is not discoverable.
                        "retry_hint": "modify or replace the file on disk to retry",
                    })),
                );
            }

            // Drop any partial converted output the failed attempt left behind.
            let _ = tokio::fs::remove_file(&conv_abs).await;

            let photo_id = Uuid::new_v4().to_string();
            let now = utc_now_iso();
            let orig_mime = crate::media::mime_from_extension(&candidate.name);
            let orig_media_type = conversion::media_type_str(candidate.target.category);

            let (img_w, img_h, orig_cam, orig_lat, orig_lon, orig_taken, orig_taken_offset) =
                extract_media_metadata_async(candidate.abs_path.clone()).await;
            let final_taken_at = orig_taken
                .map(|t| normalize_iso_timestamp(&t))
                .or(candidate.modified.clone());

            // Hash-based dedup against already-registered files.
            let photo_hash = compute_photo_hash_streaming(&candidate.abs_path).await;
            if let Some(ref hash) = photo_hash {
                let dup_exists: bool = sqlx::query_scalar(
                    "SELECT COUNT(*) > 0 FROM photos WHERE photo_hash = ? AND user_id = ?",
                )
                .bind(hash)
                .bind(admin_id)
                .fetch_one(pool)
                .await
                .unwrap_or(false);
                if dup_exists {
                    // Same deterministic dead end as the success path above, but
                    // reached after the transcode FAILED: the original's bytes
                    // are already registered under another path, so there is
                    // nothing to register and nothing to retry. Terminal, and
                    // it supersedes the attempt charged at the top of this
                    // function — burning strikes on a file whose real verdict is
                    // "duplicate" would retire it with a misleading reason.
                    crate::photos::register::record_scan_skip_path(
                        pool,
                        admin_id,
                        &candidate.rel_path,
                        candidate.size,
                        candidate.modified.as_deref(),
                        crate::photos::scan_skip::REASON_HASH_DUPLICATE,
                        Some(hash),
                        &now,
                    )
                    .await;
                    return false;
                }
            }

            // Best-effort thumbnail straight from the original — the thumbnail
            // pipeline can often read a format the browser-native conversion
            // choked on. Only record thumb_path when generation succeeds.
            let thumb_ext = if orig_mime == "image/gif" {
                "gif"
            } else {
                "jpg"
            };
            let thumb_rel = format!(".thumbnails/{photo_id}.thumb.{thumb_ext}");
            let thumb_abs = storage_root.join(&thumb_rel);
            let thumb_for_db: Option<String> = if generate_thumbnail_file(
                &candidate.abs_path,
                &thumb_abs,
                orig_mime,
                None,
            )
            .await
            {
                Some(thumb_rel)
            } else {
                None
            };

            let insert_result = sqlx::query(
                "INSERT OR IGNORE INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                 size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, \
                 created_at, photo_hash, source_path, taken_at_offset) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&photo_id)
            .bind(admin_id)
            .bind(&candidate.name)
            .bind(&candidate.rel_path)
            .bind(orig_mime)
            .bind(orig_media_type)
            .bind(candidate.size)
            .bind(img_w)
            .bind(img_h)
            .bind(&final_taken_at)
            .bind(orig_lat)
            .bind(orig_lon)
            .bind(&orig_cam)
            .bind(&thumb_for_db)
            .bind(&now)
            .bind(&photo_hash)
            .bind(&candidate.rel_path)
            .bind(&orig_taken_offset)
            .execute(pool)
            .await;
            match insert_result {
                Ok(result) if result.rows_affected() > 0 => true,
                Ok(_) => false,
                Err(err) => {
                    tracing::error!(
                        file = %candidate.name,
                        error = %err,
                        "[INGEST] Failed to register original after conversion failure"
                    );
                    false
                }
            }
        }
    }
}

/// The codec probe's decision about an "already native" container.
#[derive(Debug)]
enum OpaqueVerdict {
    /// Re-encode to browser-native MP4. `salvage` carries the decode health when
    /// the reason is a *corrupt bitstream* rather than a wrong codec, so the
    /// audit trail can warn that the output will be shorter than the source.
    Convert {
        target: conversion::ConversionTarget,
        salvage: Option<crate::transcode::probe::DecodeHealth>,
    },
    /// Already browser-native, not an opaque container, or un-probeable this pass
    /// (an environmental probe error is retried next pass, never retired).
    Leave,
    /// The container has no decodable video stream at all. A re-encode cannot
    /// invent one, so it must be retired with a terminal skip rather than
    /// re-probed on every pass forever (#46: `VIDEO0063.mp4`).
    Unplayable,
}

/// Decide whether an "already native" container actually needs converting, by
/// inspecting its streams instead of trusting its extension.
///
/// [`conversion::conversion_target`] answers from the filename alone, so every
/// `.mp4`/`.mov`/`.m4v` is assumed browser-playable and skipped. That is wrong
/// in three ways, all measured on the live library (2026-07-20):
///
/// * **Wrong codec.** An MP4 *container* happily carries HEVC, 10-bit H.264 or
///   MPEG-4 Part 2, none of which a browser can decode. 38 of 742 videos
///   (28 hevc, 10 mpeg4, one of them 10-bit) are affected and none of them has
///   ever entered the conversion queue.
/// * **Corrupt bitstream behind an intact container.** The file reported in
///   #46 probes as a flawless `h264 / Main / yuv420p` yet emits 3,331
///   `Invalid NAL unit size` errors on decode. A codec allowlist passes it.
///   Re-encoding rescues it because ffmpeg is lenient where browser decoders
///   are strict: 51s of corrupt input yields 28s of clean, playable output.
/// * **No video stream at all** (`VIDEO0063.mp4`). A re-encode cannot help, so
///   it is retired rather than retried — see [`OpaqueVerdict::Unplayable`].
async fn opaque_container_needs_conversion(abs_path: &std::path::Path, name: &str) -> OpaqueVerdict {
    use crate::transcode::probe;

    if !probe::is_opaque_video_container(name) {
        return OpaqueVerdict::Leave;
    }

    let mp4_target = conversion::ConversionTarget {
        extension: "mp4",
        mime_type: "video/mp4",
        category: conversion::MediaCategory::Video,
    };

    let info = match probe::probe_video_stream(abs_path).await {
        Ok(info) => info,
        Err(probe::ProbeError::NoVideoStream) => {
            // The container parsed but exposes nothing decodable. Retire it: the
            // caller records a terminal skip so this file stops costing an
            // ffprobe on every pass (it never registers, so `existing_set` cannot
            // cover it — this is the one file that would thrash forever).
            tracing::warn!(
                file = %abs_path.display(),
                "[INGEST] Video container has no decodable video stream — \
                 retiring as unplayable (no re-encode can invent a stream)"
            );
            return OpaqueVerdict::Unplayable;
        }
        Err(e) => {
            // Probe failure is not evidence either way. Skipping is the safe
            // default: a wrongly-queued file burns a full transcode, whereas a
            // wrongly-skipped one is retried on the next pass.
            tracing::warn!(
                file = %abs_path.display(),
                error = %e,
                "[INGEST] Could not probe video container — skipping this pass"
            );
            return OpaqueVerdict::Leave;
        }
    };

    if !probe::is_browser_native(&info) {
        tracing::info!(
            file = %abs_path.display(),
            codec = %info.codec,
            profile = ?info.profile,
            pix_fmt = ?info.pix_fmt,
            "[INGEST] Container extension says native but codec is not \
             browser-decodable — queueing for conversion"
        );
        // A wrong codec re-encodes cleanly — not a lossy salvage, so no health.
        return OpaqueVerdict::Convert {
            target: mp4_target,
            salvage: None,
        };
    }

    // Codec-native, so the only remaining question is whether the bitstream
    // behind it is real. This is the expensive check, deliberately reached
    // only after the cheap metadata probe has already cleared the file.
    match probe::probe_decode_health(abs_path).await {
        Ok(health) if !health.is_clean() => {
            tracing::warn!(
                file = %abs_path.display(),
                decode_errors = health.error_count,
                first_error = ?health.first_error,
                "[INGEST] Codec is browser-native but the bitstream does not \
                 decode — queueing a salvage re-encode (output WILL be shorter \
                 than the source; the undecodable tail is unrecoverable)"
            );
            // Carry the health so the conversion's audit row records the loss.
            OpaqueVerdict::Convert {
                target: mp4_target,
                salvage: Some(health),
            }
        }
        Ok(_) => OpaqueVerdict::Leave,
        Err(e) => {
            tracing::warn!(
                file = %abs_path.display(),
                error = %e,
                "[INGEST] Could not decode-check video — leaving as native"
            );
            OpaqueVerdict::Leave
        }
    }
}

/// One full conversion pass: walk the tree, convert non-native files, register
/// the results, and encrypt them. Always invoked under the conversion lock by
/// [`run_conversion_pass`], which also handles the rerun-on-trigger semantics.
async fn run_conversion_pass_inner(
    pool: sqlx::SqlitePool,
    storage_root: PathBuf,
    jwt_secret: String,
) {
    // ── Step 0: Determine the admin user to assign new photos to ─────────
    let admin_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();

    let admin_id = match admin_id {
        Some(id) => id,
        None => {
            tracing::debug!("[INGEST] No admin user yet, skipping conversion pass");
            return;
        }
    };

    // Panorama-detection sensitivity for the admin's imported media, resolved
    // once for this pass (item #7): precise thresholds unless AI is off.
    let pano_sensitivity =
        crate::photos::metadata::pano_sensitivity_for_user(&pool, &admin_id).await;

    // ── Wait for Phase 1 encryption to finish ────────────────────────────
    // The ingest engine must not start until ALL native files are encrypted.
    // Poll the unencrypted count with a timeout so we don't spin forever if
    // no encryption key is stored yet.
    {
        let max_wait = std::time::Duration::from_secs(300); // 5 min ceiling
        let start = std::time::Instant::now();
        loop {
            let unencrypted: i64 =
                sqlx::query_scalar(
                    "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NULL AND encryption_deferred = 0",
                )
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0);

            if unencrypted == 0 {
                break;
            }

            // If no wrapped key exists, encryption can't proceed — don't block.
            let has_key: bool = sqlx::query_scalar(
                "SELECT COUNT(*) > 0 FROM server_settings WHERE key = 'encryption_key_wrapped'",
            )
            .fetch_one(&pool)
            .await
            .unwrap_or(false);

            if !has_key {
                tracing::info!(
                    "[INGEST] No encryption key stored, proceeding with conversion \
                     ({} photos still unencrypted)",
                    unencrypted
                );
                break;
            }

            if start.elapsed() > max_wait {
                tracing::warn!(
                    "[INGEST] Timed out waiting for encryption ({} still pending), proceeding",
                    unencrypted
                );
                break;
            }

            tracing::debug!(
                "[INGEST] Waiting for {} native photos to be encrypted before starting conversion",
                unencrypted
            );
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    // Check audio-backup toggle (shared helper enforces the same policy on
    // every import path: upload, scan, ingest, autoscan, sync_engine).
    let audio_enabled: bool = crate::photos::utils::audio_backup_enabled(&pool).await;

    // ── Step 1: Build set of already-known paths ─────────────────────────
    let mut existing_set = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM photos WHERE file_path != '' \
             UNION SELECT source_path FROM photos WHERE source_path IS NOT NULL AND source_path != '' \
             UNION SELECT file_path FROM trash_items WHERE file_path != ''",
        )
        .fetch(&pool);

        while let Some(path) = rows.try_next().await.unwrap_or(None) {
            existing_set.insert(path);
        }
    }

    // Load the scan-skip cache (migration 031, extended by 038). The conversion
    // walk never consulted it before #40, so a file that fails conversion and
    // leaves no `photos` row was re-probed and re-transcoded on every pass
    // forever. Keyed by rel_path; the verdict per row is
    // `crate::photos::scan_skip::skip_verdict`, shared with the autoscan walk.
    let mut skip_map: std::collections::HashMap<String, crate::photos::scan_skip::SkipRow> =
        std::collections::HashMap::new();
    {
        let mut rows = sqlx::query_as::<_, (String, i64, Option<String>, String, i64)>(
            "SELECT rel_path, size_bytes, mtime, reason, attempt_count \
             FROM scan_skipped_paths WHERE user_id = ?",
        )
        .bind(&admin_id)
        .fetch(&pool);
        while let Some((rel_path, size_bytes, mtime, reason, attempt_count)) =
            rows.try_next().await.unwrap_or(None)
        {
            skip_map.insert(
                rel_path,
                crate::photos::scan_skip::SkipRow {
                    size_bytes,
                    mtime,
                    reason,
                    attempt_count,
                },
            );
        }
    }

    // ── Step 2: Walk directory and collect convertible candidates ─────────
    let mut candidates: Vec<ConvertCandidate> = Vec::new();
    let mut stale_skip_paths: Vec<String> = Vec::new();
    let mut retired_skipped: u64 = 0;
    let mut queue = vec![storage_root.clone()];

    while let Some(dir) = queue.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir = ?dir, error = %e, "[INGEST] Skipping unreadable directory");
                continue;
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() {
                    let abs_path = entry.path();
                    let rel_path = abs_path
                        .strip_prefix(&storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    // Skip if already processed (source_path in DB).
                    //
                    // Deliberately checked BEFORE the probe below: probing
                    // spawns ffprobe (and sometimes a bounded decode), so
                    // running it on files we already know about would put a
                    // per-video process spawn back into every idle autoscan
                    // pass — exactly the disk thrash migration 031 removed.
                    // Everything already registered is in this set, so the
                    // steady-state probe count is zero.
                    if existing_set.contains(&rel_path) {
                        continue;
                    }

                    // Size + mtime are read BEFORE the probe below, not after
                    // it as they used to be, because the skip check needs them
                    // and the whole point of that check is to happen before an
                    // ffprobe is spawned. This is one `stat` on an inode the
                    // walk has already touched via `file_type()`; the probe it
                    // guards is a process spawn plus, for opaque containers, a
                    // bounded decode.
                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            normalize_iso_timestamp(&dt.to_rfc3339())
                        })
                    });

                    // #40: a file that has burned its conversion attempts is
                    // left alone until it changes on disk. Below the cap it
                    // falls through and is retried; a file whose size or mtime
                    // moved is un-retired by dropping the stale row (migration
                    // 031's invalidation, which doubles as this cap's escape
                    // hatch).
                    if let Some(row) = skip_map.get(&rel_path) {
                        match crate::photos::scan_skip::skip_verdict(
                            row,
                            size,
                            modified.as_deref(),
                        ) {
                            crate::photos::scan_skip::SkipVerdict::Skip => {
                                retired_skipped += 1;
                                continue;
                            }
                            crate::photos::scan_skip::SkipVerdict::Stale => {
                                stale_skip_paths.push(rel_path.clone());
                            }
                            crate::photos::scan_skip::SkipVerdict::Retry => {}
                        }
                    }

                    // Extension is sufficient for formats that are *never*
                    // browser-native. For opaque containers (.mp4/.mov/...)
                    // the extension is a guess, so ask ffprobe what is
                    // actually inside — see `transcode::probe`.
                    let (target, salvage) = match conversion::conversion_target(&name) {
                        Some(t) => (t, None),
                        None => match opaque_container_needs_conversion(&abs_path, &name).await {
                            OpaqueVerdict::Convert { target, salvage } => (target, salvage),
                            OpaqueVerdict::Leave => continue,
                            OpaqueVerdict::Unplayable => {
                                // No decodable video stream (#46: VIDEO0063.mp4).
                                // Retire it with a terminal skip so the walk stops
                                // spawning an ffprobe for it on every pass — it
                                // never registers, so `existing_set` cannot cover
                                // it, and it would thrash forever otherwise. A
                                // change on disk re-evaluates it (031's rule).
                                crate::photos::register::record_scan_skip_path(
                                    &pool,
                                    &admin_id,
                                    &rel_path,
                                    size,
                                    modified.as_deref(),
                                    crate::photos::scan_skip::REASON_UNPLAYABLE,
                                    None,
                                    &utc_now_iso(),
                                )
                                .await;
                                // Surface it: an unregistered, unplayable file is
                                // invisible in the UI otherwise (the #45 complaint).
                                crate::audit::log_background(
                                    &pool,
                                    crate::audit::AuditEvent::MediaConvertFailure,
                                    Some(serde_json::json!({
                                        "filename": name,
                                        "source_path": rel_path,
                                        "category": "video",
                                        "error": "no decodable video stream",
                                        "origin": "ingest",
                                    })),
                                );
                                continue;
                            }
                        },
                    };

                    // Skip audio when toggle is off.
                    if target.category == conversion::MediaCategory::Audio && !audio_enabled {
                        continue;
                    }

                    candidates.push(ConvertCandidate {
                        abs_path,
                        rel_path,
                        name,
                        target,
                        size,
                        modified,
                        salvage,
                    });
                }
            }
        }
    }

    // Drop skip rows whose file changed on disk, so the fresh evaluation above
    // isn't shadowed next pass by a row describing bytes that are gone. Mirrors
    // the autoscan walk's handling of the same cache.
    for stale in &stale_skip_paths {
        if let Err(e) =
            sqlx::query("DELETE FROM scan_skipped_paths WHERE user_id = ? AND rel_path = ?")
                .bind(&admin_id)
                .bind(stale)
                .execute(&pool)
                .await
        {
            tracing::warn!(rel_path = %stale, error = %e, "[INGEST] Failed to clear stale scan-skip row");
        }
    }

    if retired_skipped > 0 {
        // Logged at info, not debug: this number is the difference between "the
        // server is idle" and "the server is quietly refusing to convert your
        // files", and #40 exists because that was invisible.
        tracing::info!(
            "[INGEST] Skipped {} file(s) that exhausted their conversion attempts \
             (cap {}) — they are retried if the file changes on disk",
            retired_skipped,
            crate::photos::scan_skip::CONVERSION_MAX_ATTEMPTS
        );
    }

    if candidates.is_empty() {
        tracing::debug!("[INGEST] No convertible files found");
        return;
    }

    // Convert fast formats (images, audio) before slow video transcodes so a
    // mixed import shows steady progress instead of appearing to stall on the
    // first large video (#10). `sort_by_key` is stable, so discovery order is
    // preserved within each tier.
    candidates.sort_by_key(|c| conversion::conversion_priority(c.target.category));

    let (n_img, n_aud, n_vid) = candidates
        .iter()
        .fold((0i64, 0i64, 0i64), |(i, a, v), c| match c.target.category {
            conversion::MediaCategory::Image => (i + 1, a, v),
            conversion::MediaCategory::Audio => (i, a + 1, v),
            conversion::MediaCategory::Video => (i, a, v + 1),
        });
    tracing::info!(
        "[INGEST] Found {} convertible files ({} image, {} audio, {} video) — \
         converting fast formats first",
        candidates.len(),
        n_img,
        n_aud,
        n_vid
    );

    // ── Step 3: Convert all files to .converted/ staging folder ──────────
    // The guard guarantees the global "converting" flag is cleared even if the
    // loop below panics or the task is cancelled mid-pass — otherwise a stuck
    // flag would starve the AI/geo processors forever (todo #18). We call
    // `.finish()` explicitly after the loop to preserve banner timing; the drop
    // is purely the panic/cancellation safety net.
    let batch_guard = conversion::ConversionBatchGuard::start(candidates.len() as i64);

    // Register each candidate's *work* with the weighted ETA ledger (#40). The
    // count-based estimator treats a 5 MB HEIC and a 4 GB 4K video as equal, and
    // because the sort above puts every image ahead of every video, it spends
    // the whole fast phase learning a per-item cost that is orders of magnitude
    // too small — then the ETA explodes at the video tail.
    //
    // MUST come after `ConversionBatchGuard::start`: that resets the ledger
    // along with the counters, so enqueuing first would be silently wiped.
    for c in &candidates {
        conversion::eta_enqueue(c.target.category, c.size);
    }

    let conv_dir = storage_root.join(".converted");

    // Auto-scale conversion concurrency to the host: a single-core box runs
    // everything serially, a many-core workstation runs dozens of encodes at
    // once — always leaving headroom for the rest of the server. The video lane
    // is kept separate so hardware encoders / thread-hungry libx264 don't
    // oversubscribe the cores the fast (image/audio) lane is already saturating.
    let gpu = conversion::active_hwaccel()
        .map(|h| h.is_gpu())
        .unwrap_or(false);
    let plan = conversion::detect_parallelism(gpu);
    tracing::info!(
        fast_lane = plan.fast_lane,
        video_lane = plan.video_lane,
        video_threads = plan.video_threads,
        gpu,
        "[INGEST] Conversion parallelism plan (auto-scaled to host cores)"
    );

    let ctx = ConvCtx {
        pool: pool.clone(),
        storage_root: storage_root.clone(),
        admin_id: admin_id.clone(),
        conv_dir,
        pano_sensitivity,
    };
    let registered = std::sync::atomic::AtomicI64::new(0);

    // Split fast (image/audio) from slow (video) work. Fast formats run first at
    // full width so a mixed import shows steady progress instead of appearing to
    // stall on the first big video (#10); videos then run in their own, narrower
    // lane. `partition` preserves the stable priority sort within each group.
    let (videos, fast): (Vec<&ConvertCandidate>, Vec<&ConvertCandidate>) = candidates
        .iter()
        .partition(|c| c.target.category == conversion::MediaCategory::Video);

    // Fast lane: images + audio, each ffmpeg ~single-threaded, so run as many
    // concurrently as the usable core budget allows.
    stream::iter(fast)
        .for_each_concurrent(plan.fast_lane, |candidate| {
            let ctx = &ctx;
            let registered = &registered;
            async move {
                if process_candidate(candidate, ctx, None).await {
                    registered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        })
        .await;

    // Video lane: fewer concurrent transcodes, each thread-capped so the lane
    // stays within the same core budget instead of every encode grabbing all
    // cores at once.
    stream::iter(videos)
        .for_each_concurrent(plan.video_lane, |candidate| {
            let ctx = &ctx;
            let registered = &registered;
            let video_threads = plan.video_threads;
            async move {
                if process_candidate(candidate, ctx, Some(video_threads)).await {
                    registered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        })
        .await;

    let registered = registered.load(std::sync::atomic::Ordering::Relaxed);

    batch_guard.finish();

    tracing::info!(
        "[INGEST] Conversion pass complete: {}/{} files converted and registered",
        registered,
        candidates.len()
    );

    // ── Step 5: Encrypt the newly converted files ────────────────────────
    if registered > 0 {
        tracing::info!(
            "[INGEST] Triggering encryption for {} newly converted files",
            registered
        );
        crate::photos::server_migrate::auto_migrate_after_scan(
            pool.clone(),
            storage_root,
            jwt_secret,
        )
        .await;
        tracing::info!("[INGEST] Encryption of converted files complete");

        // Converted photos were registered AFTER the scan-time burst pass
        // ran, so group them now — otherwise converted bursts never stack
        // until the next manual scan.
        if let Err(e) = crate::photos::burst::detect_bursts_for_user(&pool, &admin_id).await {
            tracing::warn!(error = %e, "[INGEST] Post-conversion burst detection failed");
        }
    }
}

#[cfg(test)]
mod phase_order_tests {
    //! Locks in the encrypt → convert → AI/geo phase ordering (item #12): AI and
    //! geo processors call [`ingest_pipeline_busy`] and defer while any earlier
    //! phase is active, so conversions never contend with encryption and
    //! post-processing never contends with either.
    use super::pipeline_busy_decision;

    #[test]
    fn idle_pipeline_is_not_busy() {
        assert!(!pipeline_busy_decision(false, false, true, false));
    }

    #[test]
    fn encryption_migration_blocks_downstream() {
        // While native files are being encrypted, AI/geo must wait.
        assert!(pipeline_busy_decision(true, false, true, false));
    }

    #[test]
    fn conversion_blocks_downstream() {
        // Conversions run after encryption; AI/geo wait until they finish too.
        assert!(pipeline_busy_decision(false, true, true, false));
    }

    #[test]
    fn pending_unencrypted_blocks_only_with_key() {
        // Files awaiting encryption keep the pipeline busy — but only when a key
        // exists to encrypt them, else a no-key install would starve AI/geo.
        assert!(pipeline_busy_decision(false, false, true, true));
        assert!(!pipeline_busy_decision(false, false, false, true));
    }
}

#[cfg(test)]
mod opaque_container_tests {
    //! End-to-end proof for #46: the conversion queue must be decided by what a
    //! video file *contains*, not by what its extension claims.
    //!
    //! These build real fixtures with FFmpeg and run the real probe, because
    //! the defect being guarded is precisely that the old extension-only path
    //! never looked inside the file. A pure unit test cannot show that.
    //! Skipped when FFmpeg is unavailable so minimal CI images stay green.
    use super::{opaque_container_needs_conversion, OpaqueVerdict};

    /// Encode a tiny fixture clip with an explicit codec. Returns `None` when
    /// FFmpeg or the requested encoder is unavailable on this host.
    fn make_fixture(name: &str, vcodec: &str, extra: &[&str]) -> Option<std::path::PathBuf> {
        let path = std::env::temp_dir().join(format!("sp_probe_{}_{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut args: Vec<String> = vec![
            "-y".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=duration=1:size=320x240:rate=10".into(),
            "-c:v".into(),
            vcodec.into(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        args.push(path.to_string_lossy().to_string());

        let ok = std::process::Command::new("ffmpeg")
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if ok && std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
            Some(path)
        } else {
            let _ = std::fs::remove_file(&path);
            None
        }
    }

    /// The 704 h264 files in the live library must stay out of the queue.
    /// Queueing them would re-encode almost the entire library for nothing.
    #[tokio::test]
    async fn native_h264_mp4_is_left_alone() {
        let Some(path) = make_fixture(
            "native.mp4",
            "libx264",
            &["-profile:v", "high", "-pix_fmt", "yuv420p"],
        ) else {
            eprintln!("ffmpeg/libx264 unavailable — skipping");
            return;
        };

        let verdict = opaque_container_needs_conversion(&path, "native.mp4").await;
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(verdict, OpaqueVerdict::Leave),
            "a browser-native H.264 MP4 must be left alone, not queued for a pointless re-encode"
        );
    }

    /// HEVC-in-MP4: 28 such files exist in the live library and none has ever
    /// been converted, because `conversion_target("x.mp4")` returns `None`.
    /// This is the assertion that fails on the pre-fix tree.
    #[tokio::test]
    async fn hevc_in_mp4_is_queued_despite_the_native_extension() {
        let Some(path) = make_fixture(
            "hevc.mp4",
            "libx265",
            &["-x265-params", "log-level=none", "-pix_fmt", "yuv420p", "-tag:v", "hvc1"],
        ) else {
            eprintln!("ffmpeg/libx265 unavailable — skipping");
            return;
        };

        // The old, extension-only answer — still what `conversion_target` says.
        assert!(
            crate::conversion::conversion_target("hevc.mp4").is_none(),
            "precondition: extension-only detection considers .mp4 native"
        );

        let verdict = opaque_container_needs_conversion(&path, "hevc.mp4").await;
        let _ = std::fs::remove_file(&path);

        let OpaqueVerdict::Convert { target, salvage } = verdict else {
            panic!("HEVC in an .mp4 container must be queued for conversion");
        };
        assert_eq!(target.extension, "mp4");
        assert_eq!(target.category, crate::conversion::MediaCategory::Video);
        assert!(
            salvage.is_none(),
            "a wrong codec re-encodes cleanly — only a corrupt bitstream is a lossy salvage"
        );
    }

    /// MPEG-4 Part 2 (DivX/Xvid era) — 10 files in the live library.
    #[tokio::test]
    async fn mpeg4_part2_in_mp4_is_queued() {
        let Some(path) = make_fixture("mpeg4.mp4", "mpeg4", &["-pix_fmt", "yuv420p"]) else {
            eprintln!("ffmpeg/mpeg4 unavailable — skipping");
            return;
        };

        let verdict = opaque_container_needs_conversion(&path, "mpeg4.mp4").await;
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(verdict, OpaqueVerdict::Convert { .. }),
            "MPEG-4 Part 2 is not browser-decodable and must be queued"
        );
    }

    /// A container that parses but exposes no video stream at all — the live
    /// library's `VIDEO0063.mp4`. It must be `Unplayable` (the caller retires it
    /// with a terminal skip), never queued: a re-encode cannot invent a stream,
    /// so queueing it would fail on every pass forever.
    #[tokio::test]
    async fn a_container_with_no_video_stream_is_unplayable() {
        let path = std::env::temp_dir()
            .join(format!("sp_probe_{}_audio_only.mp4", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:duration=1",
                "-c:a",
                "aac",
            ])
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok || std::fs::metadata(&path).map(|m| m.len() == 0).unwrap_or(true) {
            eprintln!("ffmpeg/aac unavailable — skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }

        let verdict = opaque_container_needs_conversion(&path, "VIDEO0063.mp4").await;
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(verdict, OpaqueVerdict::Unplayable),
            "an audio-only .mp4 has no video stream and must be retired, not queued"
        );
    }

    /// Formats that already convert on extension alone must never reach the
    /// probe — spawning ffprobe for them is pure waste.
    #[tokio::test]
    async fn non_opaque_extensions_are_not_probed() {
        // A path that does not exist: if the probe ran, it would error and log.
        // Returning `Leave` purely from the extension check is the correct path.
        let missing = std::path::Path::new("/nonexistent/clip.mkv");
        assert!(matches!(
            opaque_container_needs_conversion(missing, "clip.mkv").await,
            OpaqueVerdict::Leave
        ));
        assert!(matches!(
            opaque_container_needs_conversion(missing, "photo.jpg").await,
            OpaqueVerdict::Leave
        ));
        // B3b: `.mov`/`.m4v` never reach here at all — `conversion_target` claims
        // them at the call site, so the probe branch is unreachable for them.
        for name in ["clip.mov", "clip.m4v"] {
            assert!(
                crate::conversion::conversion_target(name).is_some(),
                "precondition: {name} is claimed before the probe branch"
            );
            assert!(matches!(
                opaque_container_needs_conversion(missing, name).await,
                OpaqueVerdict::Leave
            ));
        }
    }

    /// B3b: the live false positive. `.webm` is skipped by `conversion_target`,
    /// so it DID reach the probe — where `is_browser_native` is an H.264-only
    /// allowlist and could only ever answer "not native" for VP9/AV1. Every newly
    /// scanned WebM was therefore queued for a full re-encode into H.264 MP4,
    /// replacing a file every target browser plays.
    ///
    /// The inconsistency is the proof it was a bug rather than a policy: an
    /// *already-registered* `.webm` is skipped by `existing_set` and served
    /// untouched, so the same file was treated two different ways depending only
    /// on when it arrived.
    ///
    /// This asserts `Leave` and fails on the pre-fix tree, which returns
    /// `Convert`. A pure test cannot show it — the whole defect is what the probe
    /// finds inside the file.
    #[tokio::test]
    async fn a_vp9_webm_is_left_alone_rather_than_re_encoded() {
        let Some(path) = make_fixture(
            "vp9.webm",
            "libvpx-vp9",
            &["-pix_fmt", "yuv420p", "-b:v", "200k", "-deadline", "realtime", "-cpu-used", "8"],
        ) else {
            eprintln!("ffmpeg/libvpx-vp9 unavailable — skipping");
            return;
        };

        // Precondition: this file really does reach the probe branch.
        assert!(
            crate::conversion::conversion_target("vp9.webm").is_none(),
            "precondition: .webm is not claimed by extension, so it reaches the probe"
        );
        // Precondition: the allowlist genuinely cannot judge it — this is *why*
        // probing it was wrong, not merely that the answer happened to be wrong.
        let info = crate::transcode::probe::probe_video_stream(&path)
            .await
            .expect("the fixture must probe as a video");
        assert!(
            !crate::transcode::probe::is_browser_native(&info),
            "precondition: the H.264-only allowlist rejects VP9 ({info:?}) — a probe here \
             can only ever return the wrong answer"
        );

        let verdict = opaque_container_needs_conversion(&path, "vp9.webm").await;
        let _ = std::fs::remove_file(&path);

        assert!(
            matches!(verdict, OpaqueVerdict::Leave),
            "a VP9 WebM must be left alone — the pre-fix tree queued it for a full \
             re-encode of a file every target browser already plays, got {verdict:?}"
        );
    }
}
