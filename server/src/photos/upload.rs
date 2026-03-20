//! POST /api/photos/upload — mobile client photo upload handler.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::mime_from_extension;
use crate::sanitize;
use crate::state::AppState;

use super::metadata::extract_media_metadata_from_bytes_async;
use super::utils::{compute_photo_hash, normalize_iso_timestamp, utc_now_iso};

/// POST /api/photos/upload
/// Upload a photo/video/GIF file from a mobile client.
/// The file body is sent as raw bytes with metadata in custom headers:
///   X-Filename: original filename
///   X-Mime-Type: MIME type (e.g., image/jpeg)
///
/// The server stores the file in the storage root and registers it as a photo.
pub async fn upload_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}.jpg", Uuid::new_v4()));

    let mime_type = headers
        .get("X-Mime-Type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| mime_from_extension(&filename).to_string());

    let media_type = if mime_type.starts_with("video/") {
        "video"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else if mime_type == "image/gif" {
        "gif"
    } else {
        "photo"
    };

    let size_bytes = body.len() as i64;

    // Sanitize filename — strip path separators, traversal, and dangerous chars
    let safe_filename = sanitize::sanitize_filename(&filename);

    // ── Content hash for cross-platform alignment ───────────────────────
    let photo_hash = compute_photo_hash(&body);

    // ── Content-aware dedup (hash-based) ────────────────────────────────
    // If a photo with the identical content hash already exists for this
    // user, return it immediately — no duplicate stored.
    let existing: Option<(String, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, filename, file_path, size_bytes, photo_hash FROM photos \
         WHERE user_id = ? AND photo_hash = ? LIMIT 1",
    )
    .bind(&auth.user_id)
    .bind(&photo_hash)
    .fetch_optional(&state.read_pool)
    .await?;

    if let Some((eid, efn, efp, esz, ehash)) = existing {
        tracing::info!(
            user_id = %auth.user_id,
            filename = %efn,
            photo_hash = %photo_hash,
            "Duplicate upload detected (hash match) — returning existing record"
        );
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "photo_id": eid,
                "filename": efn,
                "file_path": efp,
                "size_bytes": esz,
                "photo_hash": ehash,
            })),
        ));
    }

    // Ensure unique filename if it already exists on disk (different content)
    let storage_root = (**state.storage_root.load()).clone();
    let uploads_dir = storage_root.join("uploads");
    tokio::fs::create_dir_all(&uploads_dir).await.map_err(|e| {
        AppError::Internal(format!("Failed to create uploads directory: {}", e))
    })?;

    let mut final_filename = safe_filename.clone();
    let mut counter = 1u32;
    while tokio::fs::try_exists(uploads_dir.join(&final_filename))
        .await
        .unwrap_or(false)
    {
        let stem = std::path::Path::new(&safe_filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let ext = std::path::Path::new(&safe_filename)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("jpg");
        final_filename = format!("{}-{}.{}", stem, counter, ext);
        counter += 1;
    }

    // Write file to disk
    let file_path = uploads_dir.join(&final_filename);
    tokio::fs::write(&file_path, &body).await.map_err(|e| {
        AppError::Internal(format!("Failed to write photo file: {}", e))
    })?;

    // Relative path for DB storage
    let rel_path = format!("uploads/{}", final_filename);

    // Register in database
    let photo_id = Uuid::new_v4().to_string();
    let now = utc_now_iso();
    let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

    // Extract metadata from the uploaded bytes (offloaded to spawn_blocking — CPU-bound EXIF parsing)
    let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
        extract_media_metadata_from_bytes_async(body.to_vec(), final_filename.clone()).await;

    let final_taken_at = exif_taken
        .map(|t| normalize_iso_timestamp(&t))
        .unwrap_or_else(|| now.clone());

    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
         size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, created_at, photo_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&final_filename)
    .bind(&rel_path)
    .bind(&mime_type)
    .bind(media_type)
    .bind(size_bytes)
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

    tracing::info!(
        user_id = %auth.user_id,
        filename = %final_filename,
        size = size_bytes,
        photo_hash = %photo_hash,
        "Uploaded photo via mobile client"
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "photo_id": photo_id,
            "filename": final_filename,
            "file_path": rel_path,
            "size_bytes": size_bytes,
            "photo_hash": photo_hash,
        })),
    ))
}
