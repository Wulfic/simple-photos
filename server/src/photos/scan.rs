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
use futures_util::TryStreamExt;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::metadata::extract_media_metadata_async;
use super::thumbnail::generate_thumbnail_file;
use super::utils::{compute_photo_hash_streaming, normalize_iso_timestamp, utc_now_iso};

/// Maximum concurrent file processing tasks during scan.
const SCAN_PARALLELISM: usize = 4;

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

    // Build set of already-registered paths using a streaming cursor so we
    // never hold the full Vec<String> + HashSet simultaneously in memory.
    // Include trash_items so that files deleted on the primary (which are
    // physically still on disk) are not re-imported into the gallery.
    // Include source_path so that already-converted originals are not
    // re-converted on subsequent scans.
    let mut existing_set = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM photos WHERE file_path != '' \
             UNION SELECT source_path FROM photos WHERE source_path IS NOT NULL AND source_path != '' \
             UNION SELECT file_path FROM trash_items WHERE file_path != ''"
        )
        .fetch(&state.pool);

        while let Some(path) = rows.try_next().await? {
            existing_set.insert(path);
        }
    }

    // ── Phase 1: Collect all unregistered native media files (fast directory walk) ──
    struct ScanCandidate {
        abs_path: PathBuf,
        rel_path: String,
        name: String,
        mime: String,
        media_type: &'static str,
        size: i64,
        modified: Option<String>,
    }

    let mut candidates: Vec<ScanCandidate> = Vec::new();
    let mut queue = vec![storage_root.clone()];

    while let Some(dir) = queue.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
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

                    // Native format — determine MIME and media type directly.
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

                    candidates.push(ScanCandidate {
                        abs_path,
                        rel_path,
                        name,
                        mime,
                        media_type,
                        size,
                        modified,
                    });
                }
            }
        }
    }

    // Filter out audio files when the audio-backup toggle is off.
    let audio_enabled: bool = sqlx::query_scalar(
        "SELECT value = 'true' FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(false);
    if !audio_enabled {
        candidates.retain(|c| c.media_type != "audio");
    }

    tracing::info!(
        "Scan phase 1: found {} unregistered native media files",
        candidates.len()
    );

    // ── Phase 2: Register files in parallel (metadata, hash, DB insert, thumbnail) ──
    let new_count = Arc::new(AtomicI64::new(0));
    let sem = Arc::new(Semaphore::new(SCAN_PARALLELISM));
    let mut handles = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let sem = sem.clone();
        let new_count = new_count.clone();
        let pool = state.pool.clone();
        let storage_root = storage_root.clone();
        let user_id = auth.user_id.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;

            // Native format — register directly (no conversion needed).
            let photo_id = Uuid::new_v4().to_string();
            let now = utc_now_iso();
            // GIFs get an animated GIF thumbnail; everything else gets JPEG
            let thumb_ext = if candidate.mime == "image/gif" { "gif" } else { "jpg" };
            let thumb_rel = format!(".thumbnails/{}.thumb.{}", photo_id, thumb_ext);

            // Extract dimensions, camera model, GPS, and date from file
            let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
                extract_media_metadata_async(candidate.abs_path.clone()).await;

            let final_taken_at = exif_taken
                .map(|t| normalize_iso_timestamp(&t))
                .or(candidate.modified);

            // Compute content-based hash using streaming I/O
            let photo_hash = compute_photo_hash_streaming(&candidate.abs_path).await;

            let insert_result = sqlx::query(
                "INSERT OR IGNORE INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                 size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, created_at, photo_hash) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&photo_id)
            .bind(&user_id)
            .bind(&candidate.name)
            .bind(&candidate.rel_path)
            .bind(&candidate.mime)
            .bind(candidate.media_type)
            .bind(candidate.size)
            .bind(img_w)
            .bind(img_h)
            .bind(&final_taken_at)
            .bind(exif_lat)
            .bind(exif_lon)
            .bind(&cam_model)
            .bind(&thumb_rel)
            .bind(&now)
            .bind(&photo_hash)
            .execute(&pool)
            .await;

            match insert_result {
                Ok(result) if result.rows_affected() == 0 => {
                    tracing::debug!(file = %candidate.rel_path, "Already registered (concurrent scan), skipping");
                    return;
                }
                Err(e) => {
                    tracing::error!(file = %candidate.rel_path, error = %e, "Failed to register photo");
                    return;
                }
                Ok(_) => {}
            }

            // Generate thumbnail
            let thumb_abs = storage_root.join(&thumb_rel);
            if generate_thumbnail_file(&candidate.abs_path, &thumb_abs, &candidate.mime, None).await {
                tracing::debug!(file = %candidate.rel_path, "Generated thumbnail");
            } else {
                tracing::warn!(file = %candidate.rel_path, "Failed to generate thumbnail");
            }

            new_count.fetch_add(1, Ordering::Relaxed);
        }));
    }

    // Wait for all registration tasks to complete
    for h in handles {
        let _ = h.await;
    }

    let new_count = new_count.load(Ordering::Relaxed);
    tracing::info!(
        "Scan complete: registered {} new files",
        new_count,
    );

    // ── Retroactively fill missing metadata for existing photos ──────────
    // Also re-check video dimensions: uploads prior to the ffprobe SAR fix
    // may have stored coded pixel dimensions instead of display dimensions.
    let photos_needing_fix: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, file_path, media_type FROM photos WHERE user_id = ? AND \
         (width = 0 OR height = 0 OR camera_model IS NULL OR photo_hash IS NULL \
          OR media_type = 'video')",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let fixed_count = Arc::new(AtomicI64::new(0));
    {
        let sem = Arc::new(Semaphore::new(SCAN_PARALLELISM));
        let mut handles = Vec::with_capacity(photos_needing_fix.len());

        for (pid, fpath, mtype) in photos_needing_fix {
            let abs = storage_root.join(&fpath);
            if !tokio::fs::try_exists(&abs).await.unwrap_or(false) {
                continue;
            }
            let sem = sem.clone();
            let pool = state.pool.clone();
            let fixed_count = fixed_count.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await;
                let (w, h, cam, lat, lon, taken) = extract_media_metadata_async(abs.clone()).await;
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
            }));
        }

        for h in handles {
            let _ = h.await;
        }
    }
    let fixed_count = fixed_count.load(Ordering::Relaxed);

    if fixed_count > 0 {
        tracing::info!("Updated metadata for {} existing photos", fixed_count);
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
    {
        let sem = Arc::new(Semaphore::new(SCAN_PARALLELISM));
        let mut handles = Vec::with_capacity(thumbs_to_gen.len());

        for (_pid, fpath, tpath, mime) in &thumbs_to_gen {
            let abs = storage_root.join(fpath);
            if !tokio::fs::try_exists(&abs).await.unwrap_or(false) {
                continue;
            }

            let thumb_abs = storage_root.join(tpath);
            if tokio::fs::try_exists(&thumb_abs).await.unwrap_or(false) {
                continue; // already has a thumbnail
            }

            let sem = sem.clone();
            let tc = thumb_count.clone();
            let mime = mime.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await;
                if generate_thumbnail_file(&abs, &thumb_abs, &mime, None).await {
                    tc.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }
    }

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
                pool_clone.clone(), root_clone.clone(), jwt_secret.clone(),
            ).await;
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
                pool_clone.clone(), root_clone.clone(), jwt_secret.clone(),
            ).await;
            crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_secret).await;
        });
    }

    // Run burst detection for all users after new photos are registered.
    if new_count > 0 {
        let pool_clone = state.pool.clone();
        tokio::spawn(async move {
            let users: Vec<(String,)> = match sqlx::query_as(
                "SELECT DISTINCT user_id FROM photos"
            )
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
