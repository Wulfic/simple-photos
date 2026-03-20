//! Filesystem scanning and thumbnail generation.
//!
//! `POST /api/admin/photos/scan` walks the storage directory tree, registers
//! every unregistered media file as a photo, extracts EXIF metadata,
//! and generates JPEG thumbnails using the `image` crate (pure Rust — no
//! external tools like FFmpeg or ImageMagick required).
//!
//! Only browser-native formats are accepted (see [`crate::media::MEDIA_EXTENSIONS`]).
//! No server-side format conversion is performed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

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
use super::utils::{normalize_iso_timestamp, utc_now_iso, compute_photo_hash_streaming};

/// Maximum concurrent file processing tasks during scan.
const SCAN_PARALLELISM: usize = 4;

/// Generate a JPEG thumbnail using the `image` crate (pure Rust).
///
/// Supported inputs: JPEG, PNG, GIF, WebP, BMP, ICO, AVIF.
/// For videos and audio: generates a simple placeholder image.
/// SVG: generates a generic placeholder (rasterisation would require an
/// additional crate like `resvg`; keeping it simple for now).
pub async fn generate_thumbnail_file(
    input_path: &Path,
    output_path: &Path,
    mime: &str,
    _crop_metadata: Option<&str>,
) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    // Audio → black 256×256 placeholder with no external tools
    if mime.starts_with("audio/") {
        return generate_placeholder_thumbnail(output_path, [0, 0, 0]).await;
    }

    // Video → dark-gray 256×256 placeholder (no FFmpeg available)
    if mime.starts_with("video/") {
        return generate_placeholder_thumbnail(output_path, [30, 30, 30]).await;
    }

    // SVG → placeholder (would need resvg for proper rasterisation)
    if mime == "image/svg+xml" {
        return generate_placeholder_thumbnail(output_path, [40, 40, 40]).await;
    }

    // Image formats handled by the `image` crate
    let input = input_path.to_path_buf();
    let output = output_path.to_path_buf();

    // Offload CPU-bound image decoding + resizing to a blocking thread
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let img = image::open(&input).map_err(|e| format!("Failed to open image: {}", e))?;

        // Resize to cover 256×256 then crop to exact square
        let thumb = img.resize_to_fill(256, 256, image::imageops::FilterType::Triangle);

        thumb
            .save_with_format(&output, image::ImageFormat::Jpeg)
            .map_err(|e| format!("Failed to save thumbnail: {}", e))?;

        Ok(())
    })
    .await;

    match result {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            tracing::warn!(path = %input_path.display(), error = %e, "Thumbnail generation failed");
            // Fall back to placeholder
            generate_placeholder_thumbnail(output_path, [50, 50, 50]).await
        }
        Err(e) => {
            tracing::warn!(path = %input_path.display(), error = %e, "Thumbnail task panicked");
            false
        }
    }
}

/// Generate a solid-color 256×256 JPEG placeholder thumbnail.
async fn generate_placeholder_thumbnail(output_path: &Path, color: [u8; 3]) -> bool {
    let img = image::RgbImage::from_pixel(256, 256, image::Rgb(color));
    let mut buf = std::io::Cursor::new(Vec::new());
    if image::DynamicImage::ImageRgb8(img)
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .is_ok()
    {
        return tokio::fs::write(output_path, buf.into_inner())
            .await
            .is_ok();
    }
    false
}

/// POST /api/admin/photos/scan
/// Scan the storage directory and register all unregistered media files.
///
/// For each new file: extracts EXIF metadata, generates a thumbnail, and
/// computes a content hash for deduplication.
///
/// Only browser-native formats are accepted — no conversion is performed.
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
    let mut existing_set = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM photos WHERE user_id = ?",
        )
        .bind(&auth.user_id)
        .fetch(&state.pool);

        while let Some(path) = rows.try_next().await? {
            existing_set.insert(path);
        }
    }

    // Check audio backup toggle — skip audio files when disabled
    let audio_backup_enabled: bool = sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(true); // default: enabled

    // ── Phase 1: Collect all unregistered media files (fast directory walk) ──
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
    let mut skipped_audio = 0i64;
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

                    if media_type == "audio" && !audio_backup_enabled {
                        skipped_audio += 1;
                        continue;
                    }

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

    tracing::info!("Scan phase 1: found {} unregistered media files", candidates.len());

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

            let photo_id = Uuid::new_v4().to_string();
            let now = utc_now_iso();
            let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

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

            // Generate thumbnail (pure Rust — no external tools)
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
    tracing::info!("Scan complete: registered {} new photos (skipped {} audio)", new_count, skipped_audio);

    // ── Retroactively fill missing metadata for existing photos ──────────
    let photos_needing_fix: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, file_path FROM photos WHERE user_id = ? AND (width = 0 OR height = 0 OR camera_model IS NULL OR photo_hash IS NULL)",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let fixed_count = Arc::new(AtomicI64::new(0));
    {
        let sem = Arc::new(Semaphore::new(SCAN_PARALLELISM));
        let mut handles = Vec::with_capacity(photos_needing_fix.len());

        for (pid, fpath) in photos_needing_fix {
            let abs = storage_root.join(&fpath);
            if !abs.exists() {
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
                    sqlx::query(
                        "UPDATE OR IGNORE photos SET width = CASE WHEN width = 0 THEN ? ELSE width END, \
                         height = CASE WHEN height = 0 THEN ? ELSE height END, \
                         camera_model = COALESCE(camera_model, ?), \
                         latitude = COALESCE(latitude, ?), \
                         longitude = COALESCE(longitude, ?), \
                         taken_at = COALESCE(taken_at, ?), \
                         photo_hash = COALESCE(photo_hash, ?) \
                         WHERE id = ?",
                    )
                    .bind(w)
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
            if !abs.exists() {
                continue;
            }

            let thumb_abs = storage_root.join(tpath);
            if thumb_abs.exists() {
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

    Ok(Json(serde_json::json!({
        "registered": new_count,
        "metadata_updated": fixed_count,
        "skipped_audio": skipped_audio,
        "message": format!("{} new photos registered, {} metadata updated", new_count, fixed_count),
    })))
}
