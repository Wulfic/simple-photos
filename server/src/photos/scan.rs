//! Filesystem scanning — walks the storage directory tree, registers every
//! unregistered **native** media file, extracts EXIF metadata, and generates
//! thumbnails.
//!
//! Only browser-native formats are handled here.  Non-native formats
//! (HEIC, MKV, TIFF, etc.) are converted in a separate pass by the
//! ingest engine ([`crate::ingest`]) which runs AFTER encryption of native
//! files completes — this prevents the conversion/encryption race condition.
//!
//! Thumbnail generation logic lives in [`super::thumbnail`]; web-preview
//! conversion lives in [`super::web_preview`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use futures_util::stream::{self, StreamExt};
use futures_util::TryStreamExt;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::metadata::{extract_media_metadata_async, extract_xmp_subtype};
use super::thumbnail::generate_thumbnail_file;
use super::utils::{compute_photo_hash_streaming, normalize_iso_timestamp};

/// Concurrency for the per-file registration passes.
///
/// The per-file body is deliberately memory-light — header-only metadata, a
/// streaming hash, an XMP prefix read, one `INSERT`, and a subprocess thumbnail
/// — so throughput scales with CPU cores. The old fixed `4` throttled large
/// (100GB+) imports to many hours. Encryption is a SEPARATE, memory-budgeted
/// pass, so raising this does NOT add to the decode/OOM pressure the
/// conservative value was guarding against. Clamped to `[4, 16]` so small boxes
/// don't overcommit and large boxes don't spawn an unbounded thundering herd of
/// `ffmpeg` thumbnail processes.
pub(crate) fn scan_parallelism() -> usize {
    num_cpus::get().clamp(4, 16)
}

/// For each new file: extracts EXIF metadata, generates a thumbnail, and
/// computes a content hash for deduplication.
///
/// Only browser-native formats are registered here.  Non-native formats
/// are handled by the ingest engine after encryption completes.
///
/// Uses `INSERT OR IGNORE` for graceful handling of concurrent scans.
/// Original files are **never modified or deleted** by this endpoint.
pub async fn scan_and_register(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    // Serialize scan operations to prevent concurrent scans from racing.
    let _scan_guard = state.scan_lock.lock().await;

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();

    // Panorama-detection sensitivity for this scan, resolved once (item #7):
    // precise thresholds unless the user turned AI categorisation off.
    let pano_sensitivity =
        super::metadata::pano_sensitivity_for_user(&state.read_pool, &auth.user_id).await;

    // Build set of already-registered paths using a streaming cursor so we
    // never hold the full Vec<String> + HashSet simultaneously in memory.
    // Include trash_items so that files deleted on the primary (which are
    // physically still on disk) are not re-imported into the gallery.
    // `original_file_path` covers encrypted-blob deletions, where file_path is
    // the blob storage_path and the deleted photo's plaintext original (kept on
    // disk by the encryption step) would otherwise be re-imported (#3).
    // Include source_path so that already-converted originals are not
    // re-converted on subsequent scans.
    let mut existing_set = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM photos WHERE file_path != '' \
             UNION SELECT source_path FROM photos WHERE source_path IS NOT NULL AND source_path != '' \
             UNION SELECT file_path FROM trash_items WHERE file_path != '' \
             UNION SELECT original_file_path FROM trash_items WHERE original_file_path IS NOT NULL AND original_file_path != ''"
        )
        .fetch(&state.pool);

        while let Some(path) = rows.try_next().await? {
            existing_set.insert(path);
        }
    }

    // ── Phase 1: Collect all unregistered native media files (fast directory walk) ──
    // The per-file registration body is shared with the background / bulk-import
    // autoscan through crate::photos::register (single source of truth).
    use crate::photos::register::NativeCandidate;

    let mut candidates: Vec<NativeCandidate> = Vec::new();
    let mut queue = vec![storage_root.clone()];

    while let Some(dir) = queue.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Candidates registered in THIS directory + the `.json` sidecars beside
        // them, so Google Takeout metadata (capture date, GPS, album) can be
        // paired below. A streaming walk can't look ahead to a sidecar that
        // sorts after its media file, so we resolve once the dir is fully read.
        let dir_start = candidates.len();
        let mut json_names: Vec<String> = Vec::new();

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() && name.to_lowercase().ends_with(".json") {
                    json_names.push(name);
                } else if ft.is_file() && is_media_file(&name) {
                    let abs_path = entry.path();
                    let rel_path = abs_path
                        .strip_prefix(&storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    if existing_set.contains(&rel_path) {
                        continue;
                    }

                    // Native format — determine MIME and media type directly. A
                    // content-based GIF rescue happens later in `register_native_file`
                    // (it reads the header once with the file already open).
                    let mime = mime_from_extension(&name).to_string();
                    let media_type: &'static str = if mime.starts_with("video/") {
                        "video"
                    } else if mime.starts_with("audio/") {
                        "audio"
                    } else if mime == "image/gif" {
                        "gif"
                    } else {
                        "photo"
                    };

                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            normalize_iso_timestamp(&dt.to_rfc3339())
                        })
                    });

                    candidates.push(NativeCandidate {
                        abs_path,
                        rel_path,
                        name,
                        mime,
                        media_type,
                        size,
                        modified,
                        sidecar_abs: None,
                        album_name: None,
                    });
                }
            }
        }

        // Pair each of this dir's media files with its Takeout sidecar + album.
        if !json_names.is_empty() {
            let ctx = crate::import::sidecar::TakeoutDirContext::new(json_names, &dir);
            let album = ctx.album_name().map(|s| s.to_string());
            for cand in &mut candidates[dir_start..] {
                cand.sidecar_abs = ctx.resolve_sidecar(&cand.name).map(|j| dir.join(j));
                cand.album_name = album.clone();
            }
        }
    }

    // Filter out audio files when the audio-backup toggle is off.
    if !super::utils::audio_backup_enabled(&state.pool).await {
        candidates.retain(|c| c.media_type != "audio");
    }

    // Google Photos Takeout dedup (#19): within each directory, drop the
    // unedited original when its baked-in "-edited" sibling was also collected —
    // keep the edited pixels. Same shared rule as the autoscan + ingest paths
    // (crate::media::edited_shadowed_originals), scoped per directory because
    // Takeout always ships the original and its edited copy side by side.
    {
        use std::collections::{HashMap, HashSet};
        let mut names_by_dir: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for c in &candidates {
            let dir = c
                .abs_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            names_by_dir.entry(dir).or_default().push(c.name.clone());
        }
        let mut drop_keys: HashSet<(PathBuf, String)> = HashSet::new();
        for (dir, names) in &names_by_dir {
            for orig in crate::media::edited_shadowed_originals(names.iter().map(|s| s.as_str())) {
                drop_keys.insert((dir.clone(), orig));
            }
        }
        if !drop_keys.is_empty() {
            let before = candidates.len();
            candidates.retain(|c| {
                let dir = c
                    .abs_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_default();
                !drop_keys.contains(&(dir, c.name.to_lowercase()))
            });
            let dropped = before - candidates.len();
            if dropped > 0 {
                tracing::info!(
                    dropped,
                    "Scan: skipped unedited Google Photos originals with an '-edited' sibling (#19)"
                );
            }
        }
    }

    tracing::info!(
        "Scan phase 1: found {} unregistered native media files",
        candidates.len()
    );

    // ── Phase 2: Register files with bounded concurrency ──
    // A buffered stream caps live per-file futures at SCAN_PARALLELISM rather
    // than spawning one task per candidate up front; on a large library that
    // up-front fan-out was a heap spike that could OOM the process. The inner
    // spawn preserves multi-core parallelism and isolates per-file panics.
    // Content hashes of gallery-hidden originals (secure gallery) to exclude, so
    // `/scan` can never re-import a secure item's plaintext original — the same
    // protection the background autoscan already applied (previously missing
    // here, a divergence between the two hand-copied registration bodies).
    let mut gallery_hashes = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT original_photo_hash FROM encrypted_gallery_items WHERE original_photo_hash IS NOT NULL",
        )
        .fetch(&state.pool);
        while let Some(hash) = rows.try_next().await? {
            gallery_hashes.insert(hash);
        }
    }

    let ctx = Arc::new(crate::photos::register::RegisterContext {
        user_id: auth.user_id.clone(),
        pano_sensitivity,
        gallery_hashes: Arc::new(gallery_hashes),
    });

    let new_count = Arc::new(AtomicI64::new(0));
    stream::iter(candidates)
        .map(|candidate| {
            let new_count = new_count.clone();
            let pool = state.pool.clone();
            let storage_root = storage_root.clone();
            let ctx = ctx.clone();
            async move {
                // Inner spawn keeps multi-core parallelism and isolates a
                // per-file panic from the rest of the pass.
                let _ = tokio::spawn(async move {
                    if crate::photos::register::register_native_file(
                        &pool,
                        &storage_root,
                        &candidate,
                        &ctx,
                    )
                    .await
                    {
                        new_count.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .await;
            }
        })
        .buffer_unordered(scan_parallelism())
        .for_each(|_| async {})
        .await;

    let new_count = new_count.load(Ordering::Relaxed);
    tracing::info!("Scan complete: registered {} new files", new_count,);

    // ── Retroactively fill missing metadata for existing photos ──────────
    // Also re-check video dimensions ONCE per user: uploads prior to the
    // ffprobe SAR fix may have stored coded pixel dimensions instead of
    // display dimensions.  Without the one-time flag, every scan re-hashed
    // and re-probed every video in the library forever.
    let video_repair_key = format!("video_dim_repair_v1:{}", auth.user_id);
    let video_repair_done: bool =
        sqlx::query_scalar("SELECT value = 'true' FROM server_settings WHERE key = ?")
            .bind(&video_repair_key)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
            .unwrap_or(false);

    let fix_query = if video_repair_done {
        "SELECT id, file_path, media_type FROM photos WHERE user_id = ? AND \
         (width = 0 OR height = 0 OR camera_model IS NULL OR photo_hash IS NULL)"
    } else {
        "SELECT id, file_path, media_type FROM photos WHERE user_id = ? AND \
         (width = 0 OR height = 0 OR camera_model IS NULL OR photo_hash IS NULL \
          OR media_type = 'video')"
    };
    let photos_needing_fix: Vec<(String, String, String)> = sqlx::query_as(fix_query)
        .bind(&auth.user_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    let fixed_count = Arc::new(AtomicI64::new(0));
    stream::iter(photos_needing_fix)
        .map(|(pid, fpath, mtype)| {
            let pool = state.pool.clone();
            let fixed_count = fixed_count.clone();
            let storage_root = storage_root.clone();
            async move {
                let abs = storage_root.join(&fpath);
                if !tokio::fs::try_exists(&abs).await.unwrap_or(false) {
                    return;
                }
                let _ = tokio::spawn(async move {
                let (w, h, cam, lat, lon, taken, taken_offset) =
                    extract_media_metadata_async(abs.clone()).await;
                let file_hash = compute_photo_hash_streaming(&abs).await;

                if w > 0 || h > 0 || cam.is_some() || lat.is_some() || file_hash.is_some() {
                    // For videos, always overwrite dimensions: earlier uploads
                    // may have stored coded pixel dimensions (imagesize) instead
                    // of display dimensions (ffprobe with SAR correction).
                    let is_video = mtype == "video";
                    let (bind_w, bind_h) = if is_video && w > 0 && h > 0 {
                        (w, h)
                    } else {
                        (0, 0)  // sentinel: "only write if current is 0"
                    };
                    sqlx::query(
                        "UPDATE OR IGNORE photos SET \
                         width = CASE WHEN ? > 0 THEN ? WHEN width = 0 THEN ? ELSE width END, \
                         height = CASE WHEN ? > 0 THEN ? WHEN height = 0 THEN ? ELSE height END, \
                         camera_model = COALESCE(camera_model, ?), \
                         latitude = COALESCE(latitude, ?), \
                         longitude = COALESCE(longitude, ?), \
                         taken_at = COALESCE(taken_at, ?), \
                         taken_at_offset = COALESCE(taken_at_offset, ?), \
                         photo_hash = COALESCE(photo_hash, ?) \
                         WHERE id = ?",
                    )
                    .bind(bind_w)  // video override flag
                    .bind(w)      // video override value
                    .bind(w)      // fallback for width = 0
                    .bind(bind_h)
                    .bind(h)
                    .bind(h)
                    .bind(&cam)
                    .bind(lat)
                    .bind(lon)
                    .bind(&taken)
                    .bind(&taken_offset)
                    .bind(&file_hash)
                    .bind(&pid)
                    .execute(&pool)
                    .await
                    .map_err(|e| {
                        tracing::warn!(photo_id = %pid, error = %e, "Failed to update photo metadata during scan");
                        e
                    })
                    .ok();
                    fixed_count.fetch_add(1, Ordering::Relaxed);
                }
                })
                .await;
            }
        })
        .buffer_unordered(scan_parallelism())
        .for_each(|_| async {})
        .await;
    let fixed_count = fixed_count.load(Ordering::Relaxed);

    if fixed_count > 0 {
        tracing::info!("Updated metadata for {} existing photos", fixed_count);
    }

    // Mark the one-time per-user video dimension repair as done so later
    // scans stop re-hashing/re-probing every video in the library.
    if !video_repair_done {
        if let Err(e) = sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES (?, 'true') \
             ON CONFLICT(key) DO UPDATE SET value = 'true'",
        )
        .bind(&video_repair_key)
        .execute(&state.pool)
        .await
        {
            tracing::warn!(error = %e, "Failed to persist video dimension repair flag");
        }
    }

    // ── One-time canonicalization of legacy timestamps (#13) ─────────────
    // Gallery order is a *string* `ORDER BY COALESCE(taken_at, created_at) DESC`,
    // so every value must be the canonical `YYYY-MM-DDTHH:MM:SS.sssZ` form. Older
    // code versions (and the pre-fix Takeout endpoint) wrote raw EXIF
    // ("2021:06:01 12:00:00"), offset ("…+00:00"), or micros-precision strings;
    // those sort incorrectly against canonical rows, scrambling the timeline.
    // All current write paths already normalize, so this only rewrites the
    // legacy stragglers, runs once per user, and is a no-op on a clean library.
    let ts_repair_key = format!("taken_at_canonical_repair_v1:{}", auth.user_id);
    let ts_repair_done: bool =
        sqlx::query_scalar("SELECT value = 'true' FROM server_settings WHERE key = ?")
            .bind(&ts_repair_key)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
            .unwrap_or(false);
    if !ts_repair_done {
        let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, taken_at, created_at FROM photos WHERE user_id = ?",
        )
        .bind(&auth.user_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        let mut repaired = 0usize;
        for (pid, taken_at, created_at) in rows {
            // Only a non-empty value that changes under normalization needs a write.
            let new_taken = taken_at.as_deref().and_then(|t| {
                let t = t.trim();
                let n = normalize_iso_timestamp(t);
                (!t.is_empty() && n != t).then_some(n)
            });
            let new_created = created_at.as_deref().and_then(|c| {
                let c = c.trim();
                let n = normalize_iso_timestamp(c);
                (!c.is_empty() && n != c).then_some(n)
            });
            if new_taken.is_none() && new_created.is_none() {
                continue;
            }
            // COALESCE keeps the untouched column unchanged.
            if let Err(e) = sqlx::query(
                "UPDATE photos SET taken_at = COALESCE(?, taken_at), \
                 created_at = COALESCE(?, created_at) WHERE id = ?",
            )
            .bind(&new_taken)
            .bind(&new_created)
            .bind(&pid)
            .execute(&state.pool)
            .await
            {
                tracing::warn!(photo_id = %pid, error = %e, "Failed to canonicalize legacy timestamp");
            } else {
                repaired += 1;
            }
        }

        if repaired > 0 {
            tracing::info!(
                repaired,
                "Canonicalized {} legacy photo timestamps for correct ordering (#13)",
                repaired
            );
        }
        if let Err(e) = sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES (?, 'true') \
             ON CONFLICT(key) DO UPDATE SET value = 'true'",
        )
        .bind(&ts_repair_key)
        .execute(&state.pool)
        .await
        {
            tracing::warn!(error = %e, "Failed to persist timestamp canonicalization flag");
        }
    }

    // ── One-time content-based GIF re-detection for existing photos (#14) ─
    // Earlier imports classified media purely by extension/MIME, so GIFs that
    // arrived renamed (`funny.jpg`), with a generic MIME, or oddly exported by
    // Takeout were tagged `photo` and never showed in the GIF smart album. Sniff
    // the leading bytes of every `photo` row once per user and re-tag the real
    // GIFs. Runs once (gated) and is a no-op on a library with no hidden GIFs.
    let gif_repair_key = format!("gif_detect_repair_v1:{}", auth.user_id);
    let gif_repair_done: bool =
        sqlx::query_scalar("SELECT value = 'true' FROM server_settings WHERE key = ?")
            .bind(&gif_repair_key)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
            .unwrap_or(false);
    if !gif_repair_done {
        let photo_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, file_path FROM photos WHERE user_id = ? AND media_type = 'photo' \
             AND file_path != ''",
        )
        .bind(&auth.user_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        let regif_count = Arc::new(AtomicI64::new(0));
        stream::iter(photo_rows)
            .map(|(pid, fpath)| {
                let pool = state.pool.clone();
                let regif_count = regif_count.clone();
                let storage_root = storage_root.clone();
                async move {
                    let abs = storage_root.join(&fpath);
                    let header = match crate::photos::register::read_header_bytes(&abs).await {
                        Some(h) => h,
                        None => return,
                    };
                    if crate::media::gif_override("photo", &header).is_none() {
                        return;
                    }
                    if let Err(e) = sqlx::query(
                        "UPDATE photos SET media_type = 'gif', mime_type = 'image/gif' WHERE id = ?",
                    )
                    .bind(&pid)
                    .execute(&pool)
                    .await
                    {
                        tracing::warn!(photo_id = %pid, error = %e, "Failed to re-tag GIF during scan (#14)");
                    } else {
                        regif_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
            .buffer_unordered(scan_parallelism())
            .for_each(|_| async {})
            .await;

        let regif_count = regif_count.load(Ordering::Relaxed);
        if regif_count > 0 {
            tracing::info!(
                regif_count,
                "Re-tagged {} misclassified GIFs from content signature (#14)",
                regif_count
            );
        }
        if let Err(e) = sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES (?, 'true') \
             ON CONFLICT(key) DO UPDATE SET value = 'true'",
        )
        .bind(&gif_repair_key)
        .execute(&state.pool)
        .await
        {
            tracing::warn!(error = %e, "Failed to persist GIF detection repair flag");
        }
    }

    // ── Retroactively fill missing XMP subtypes for existing photos ──────
    // Detect motion, panorama, burst, HDR subtypes that were missed by
    // earlier scan versions that didn't do XMP extraction.
    let photos_needing_subtype: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, file_path, COALESCE(width, 0), COALESCE(height, 0) FROM photos \
         WHERE user_id = ? AND photo_subtype IS NULL AND file_path != '' \
         AND media_type = 'photo'",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    if !photos_needing_subtype.is_empty() {
        tracing::info!(
            "Checking {} existing photos for XMP subtypes",
            photos_needing_subtype.len()
        );
        let subtype_count = Arc::new(AtomicI64::new(0));
        stream::iter(photos_needing_subtype)
        .map(|(pid, fpath, ph_w, ph_h)| {
            let pool = state.pool.clone();
            let user_id = auth.user_id.clone();
            let storage_root = storage_root.clone();
            let subtype_count = subtype_count.clone();
            async move {
                let abs = storage_root.join(&fpath);
                if !tokio::fs::try_exists(&abs).await.unwrap_or(false) {
                    return;
                }
                let _ = tokio::spawn(async move {
                let file_bytes = super::metadata::read_file_prefix(
                    &abs,
                    super::metadata::XMP_SCAN_PREFIX_BYTES,
                )
                .await;
                let mut sub = extract_xmp_subtype(&file_bytes);

                // If the photos row never recorded width/height (legacy
                // imports), the aspect fallback would otherwise skip these
                // panoramas forever.  Re-extract dimensions from disk.
                let (mut eff_w, mut eff_h) = (ph_w, ph_h);
                if eff_w <= 0 || eff_h <= 0 {
                    let fname = abs
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    let (rw, rh, _, _, _, _, _) =
                        super::metadata::extract_media_metadata_from_bytes_async(
                            file_bytes.clone(),
                            fname,
                        )
                        .await;
                    if rw > 0 && rh > 0 {
                        eff_w = rw;
                        eff_h = rh;
                        // Persist the re-extracted dimensions so future
                        // queries (and aspect-aware previews) see them.
                        let _ = sqlx::query(
                            "UPDATE photos SET width = ?, height = ? WHERE id = ?",
                        )
                        .bind(eff_w)
                        .bind(eff_h)
                        .bind(&pid)
                        .execute(&pool)
                        .await;
                    }
                }

                // Aspect-ratio fallback for panoramas / 360° photos
                // missing XMP markers (e.g. scanned or re-exported files).
                super::metadata::apply_aspect_subtype_fallback_with(
                    &mut sub,
                    eff_w,
                    eff_h,
                    pano_sensitivity,
                );

                if let Some(ref subtype) = sub.photo_subtype {
                    sqlx::query(
                        "UPDATE photos SET photo_subtype = ?, burst_id = COALESCE(burst_id, ?) WHERE id = ?",
                    )
                    .bind(subtype)
                    .bind(&sub.burst_id)
                    .bind(&pid)
                    .execute(&pool)
                    .await
                    .ok();

                    // Extract motion video blob if applicable.  The trailer
                    // lives at end-of-file, so this is the one case that
                    // needs the full bytes (bounded: motion photos are stills).
                    if subtype == "motion" {
                        let full_bytes = tokio::fs::read(&abs).await.unwrap_or_default();
                        if !full_bytes.is_empty() {
                            super::motion::extract_and_store_motion_video(
                                &pool,
                                &storage_root,
                                &user_id,
                                &pid,
                                &full_bytes,
                                sub.motion_video_offset,
                            )
                            .await;
                        }
                    }

                    tracing::info!(
                        photo_id = %pid,
                        photo_subtype = %subtype,
                        "Scan: retroactively detected XMP subtype"
                    );
                    subtype_count.fetch_add(1, Ordering::Relaxed);
                }
                })
                .await;
            }
        })
        .buffer_unordered(scan_parallelism())
        .for_each(|_| async {})
        .await;

        let sc = subtype_count.load(Ordering::Relaxed);
        if sc > 0 {
            tracing::info!("Retroactively detected {} photo subtypes", sc);
        }
    }

    // ── Generate missing thumbnails for existing photos ──────────────────
    let thumbs_to_gen: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, file_path, thumb_path, mime_type FROM photos WHERE user_id = ? AND thumb_path IS NOT NULL",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let thumb_count = Arc::new(AtomicI64::new(0));
    stream::iter(thumbs_to_gen)
        .map(|(_pid, fpath, tpath, mime)| {
            let tc = thumb_count.clone();
            let storage_root = storage_root.clone();
            async move {
                let abs = storage_root.join(&fpath);
                if !tokio::fs::try_exists(&abs).await.unwrap_or(false) {
                    return;
                }
                let thumb_abs = storage_root.join(&tpath);
                if tokio::fs::try_exists(&thumb_abs).await.unwrap_or(false) {
                    return; // already has a thumbnail
                }
                let _ = tokio::spawn(async move {
                    if generate_thumbnail_file(&abs, &thumb_abs, &mime, None).await {
                        tc.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .await;
            }
        })
        .buffer_unordered(scan_parallelism())
        .for_each(|_| async {})
        .await;

    let tc = thumb_count.load(Ordering::Relaxed);
    if tc > 0 {
        tracing::info!("Generated {} missing thumbnails", tc);
    }

    // Trigger encryption migration for any newly registered (unencrypted) photos,
    // then run the conversion ingest engine for non-native files.
    // Sequencing: native encrypt FIRST → conversion → encrypt converted.
    if new_count > 0 {
        let pool_clone = state.pool.clone();
        let root_clone = storage_root.clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            // Phase 1: Encrypt native files
            crate::photos::server_migrate::auto_migrate_after_scan(
                pool_clone.clone(),
                root_clone.clone(),
                jwt_secret.clone(),
            )
            .await;
            // Phase 2: Convert non-native files, register, then encrypt those
            crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_secret).await;
        });
    } else {
        // Even if no native files were found, there may be convertible files.
        // Still run auto_migrate to encrypt any stale unencrypted photos
        // (e.g. from prior uploads) so the conversion wait loop doesn't block.
        let pool_clone = state.pool.clone();
        let root_clone = storage_root.clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            crate::photos::server_migrate::auto_migrate_after_scan(
                pool_clone.clone(),
                root_clone.clone(),
                jwt_secret.clone(),
            )
            .await;
            crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_secret).await;
        });
    }

    // Run burst detection for all users after new photos are registered.
    if new_count > 0 {
        let pool_clone = state.pool.clone();
        tokio::spawn(async move {
            let users: Vec<(String,)> = match sqlx::query_as("SELECT DISTINCT user_id FROM photos")
                .fetch_all(&pool_clone)
                .await
            {
                Ok(u) => u,
                Err(e) => {
                    tracing::warn!("Burst detection: failed to list users: {}", e);
                    return;
                }
            };
            for (user_id,) in &users {
                if let Err(e) = super::burst::detect_bursts_for_user(&pool_clone, user_id).await {
                    tracing::warn!("Burst detection failed for user {}: {}", user_id, e);
                }
            }
        });
    }

    Ok(Json(serde_json::json!({
        "registered": new_count,
        "metadata_updated": fixed_count,
        "message": format!("{} new files registered, {} metadata updated", new_count, fixed_count),
    })))
}
