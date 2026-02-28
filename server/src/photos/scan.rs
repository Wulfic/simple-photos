use axum::extract::State;
use axum::Json;
use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::state::AppState;

use super::metadata::extract_media_metadata;

/// Compute short content-based hash: first 12 hex chars of SHA-256.
fn compute_photo_hash(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex::encode(&digest[..6])
}

/// POST /api/admin/photos/scan
/// Scan the storage directory and register all unregistered media files as plain photos.
/// This is the main "import" mechanism for plain mode.
pub async fn scan_and_register(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify admin
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let storage_root = state.storage_root.read().await.clone();

    // Get already-registered file paths
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT file_path FROM photos WHERE user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;
    let existing_set: std::collections::HashSet<String> = existing.into_iter().collect();

    // Scan recursively for media files
    let mut new_count = 0i64;
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
                    // Normalize to forward slashes so DB paths are consistent across OS
                    let rel_path = abs_path
                        .strip_prefix(&storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    if existing_set.contains(&rel_path) {
                        continue; // Already registered
                    }

                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            dt.to_rfc3339()
                        })
                    });

                    let mime = mime_from_extension(&name).to_string();
                    let media_type = if mime.starts_with("video/") {
                        "video"
                    } else if mime == "image/gif" {
                        "gif"
                    } else {
                        "photo"
                    };

                    let photo_id = Uuid::new_v4().to_string();
                    let now = Utc::now().to_rfc3339();
                    let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

                    // Extract dimensions, camera model, GPS, and date from file
                    let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
                        extract_media_metadata(&abs_path);

                    // Use EXIF taken_at if available, otherwise fall back to file modified time
                    let final_taken_at = exif_taken.or(modified);

                    // Compute content-based hash for cross-platform alignment
                    let photo_hash = match tokio::fs::read(&abs_path).await {
                        Ok(bytes) => Some(compute_photo_hash(&bytes)),
                        Err(_) => None,
                    };

                    sqlx::query(
                        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                         size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, created_at, photo_hash) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(&photo_id)
                    .bind(&auth.user_id)
                    .bind(&name)
                    .bind(&rel_path)
                    .bind(&mime)
                    .bind(media_type)
                    .bind(size)
                    .bind(img_w)
                    .bind(img_h)
                    .bind(&final_taken_at)
                    .bind(exif_lat)
                    .bind(exif_lon)
                    .bind(&cam_model)
                    .bind(&thumb_rel)
                    .bind(&now)
                    .bind(&photo_hash)
                    .execute(&state.pool)
                    .await?;

                    new_count += 1;
                }
            }
        }
    }

    tracing::info!("Scan complete: registered {} new photos", new_count);

    // ── Retroactively fill missing metadata for existing photos ──────────
    // Fix photos with 0×0 dimensions or missing camera_model/GPS/photo_hash
    let photos_needing_fix: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, file_path FROM photos WHERE user_id = ? AND (width = 0 OR height = 0 OR camera_model IS NULL OR photo_hash IS NULL)",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut fixed_count = 0i64;
    for (pid, fpath) in &photos_needing_fix {
        let abs = storage_root.join(fpath);
        if !abs.exists() {
            continue;
        }
        let (w, h, cam, lat, lon, taken) = extract_media_metadata(&abs);

        // Compute content hash if missing
        let file_hash = match tokio::fs::read(&abs).await {
            Ok(bytes) => Some(compute_photo_hash(&bytes)),
            Err(_) => None,
        };

        if w > 0 || h > 0 || cam.is_some() || lat.is_some() || file_hash.is_some() {
            sqlx::query(
                "UPDATE photos SET width = CASE WHEN width = 0 THEN ? ELSE width END, \
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
            .bind(pid)
            .execute(&state.pool)
            .await
            .ok();
            fixed_count += 1;
        }
    }

    if fixed_count > 0 {
        tracing::info!("Updated metadata for {} existing photos", fixed_count);
    }

    Ok(Json(serde_json::json!({
        "registered": new_count,
        "metadata_updated": fixed_count,
        "message": format!("{} new photos registered, {} metadata updated", new_count, fixed_count),
    })))
}
