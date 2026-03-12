use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use super::models::*;
use super::utils::{compute_photo_hash, normalize_iso_timestamp, utc_now_iso};

// ── Plain Photo Endpoints ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PhotoListQuery {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub media_type: Option<String>,
    pub favorites_only: Option<bool>,
}

/// GET /api/photos
/// List plain-mode photos for the authenticated user.
pub async fn list_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<PhotoListQuery>,
) -> Result<Json<PhotoListResponse>, AppError> {
    let limit = params.limit.unwrap_or(100).min(500);
    let fav_only = params.favorites_only.unwrap_or(false);

    // Build dynamic query
    let mut sql = String::from(
        "SELECT id, filename, file_path, mime_type, media_type, size_bytes, width, height, \
         duration_secs, taken_at, latitude, longitude, thumb_path, created_at, is_favorite, crop_metadata, camera_model, photo_hash \
         FROM photos WHERE user_id = ? AND encrypted_blob_id IS NULL"
    );
    let mut binds: Vec<String> = vec![auth.user_id.clone()];

    if let Some(ref mt) = params.media_type {
        sql.push_str(" AND media_type = ?");
        binds.push(mt.clone());
    }

    if fav_only {
        sql.push_str(" AND is_favorite = 1");
    }

    if let Some(ref after) = params.after {
        sql.push_str(" AND COALESCE(taken_at, created_at) < ?");
        binds.push(after.clone());
    }

    sql.push_str(" ORDER BY COALESCE(taken_at, created_at) DESC, filename ASC LIMIT ?");
    binds.push((limit + 1).to_string());

    let mut query = sqlx::query_as::<_, PhotoRecord>(&sql);
    for (i, val) in binds.iter().enumerate() {
        if i == binds.len() - 1 {
            // Last bind is the limit (integer)
            query = query.bind(val.parse::<i64>().unwrap_or(limit + 1));
        } else {
            query = query.bind(val);
        }
    }

    let photos = query.fetch_all(&state.pool).await?;

    let next_cursor = if photos.len() as i64 > limit {
        photos.last().map(|p| p.taken_at.clone().unwrap_or_else(|| p.created_at.clone()))
    } else {
        None
    };

    let photos: Vec<PhotoRecord> = photos.into_iter().take(limit as usize).collect();

    Ok(Json(PhotoListResponse {
        photos,
        next_cursor,
    }))
}

/// POST /api/photos/register
/// Register a plain file on disk as a photo in the database.
/// The file must already exist at the given path within the storage root.
pub async fn register_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<RegisterPhotoRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Security: ensure file_path is a safe relative path (no traversal, no absolute)
    sanitize::validate_relative_path(&req.file_path).map_err(|reason| {
        AppError::BadRequest(format!("Invalid file_path: {}", reason))
    })?;

    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&req.file_path);

    // Verify the file actually exists
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err(AppError::BadRequest(format!(
            "File does not exist: {}",
            req.file_path
        )));
    }

    let photo_id = Uuid::new_v4().to_string();
    let now = utc_now_iso();
    let media_type = req.media_type.unwrap_or_else(|| {
        if req.mime_type.starts_with("video/") {
            "video".to_string()
        } else if req.mime_type.starts_with("audio/") {
            "audio".to_string()
        } else if req.mime_type == "image/gif" {
            "gif".to_string()
        } else {
            "photo".to_string()
        }
    });

    // Compute content-based hash for cross-platform alignment
    let file_bytes = tokio::fs::read(&full_path).await.map_err(|e| {
        AppError::Internal(format!("Failed to read file for hashing: {}", e))
    })?;
    let photo_hash = compute_photo_hash(&file_bytes);

    // Generate thumbnail path (will be created by a separate endpoint/process)
    let thumb_filename = format!("{}.thumb.jpg", photo_id);
    let thumb_rel = format!(".thumbnails/{}", thumb_filename);

    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path, created_at, photo_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&req.filename)
    .bind(&req.file_path)
    .bind(&req.mime_type)
    .bind(&media_type)
    .bind(req.size_bytes)
    .bind(req.width.unwrap_or(0))
    .bind(req.height.unwrap_or(0))
    .bind(req.duration_secs)
    .bind(req.taken_at.as_ref().map(|t| normalize_iso_timestamp(t)))
    .bind(req.latitude)
    .bind(req.longitude)
    .bind(&thumb_rel)
    .bind(&now)
    .bind(&photo_hash)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "photo_id": photo_id,
            "thumb_path": thumb_rel,
            "photo_hash": photo_hash,
        })),
    ))
}

/// GET /api/photos/:id/file
/// Serve the original (unencrypted) photo file from disk.
pub async fn serve_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    let (file_path, mime_type, size_bytes): (String, String, i64) = sqlx::query_as(
        "SELECT file_path, mime_type, size_bytes FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| {
        tracing::warn!(
            user_id = %auth.user_id,
            photo_id = %photo_id,
            "serve_photo: photo not found in database"
        );
        AppError::NotFound
    })?;

    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&file_path);

    tracing::debug!(
        user_id = %auth.user_id,
        photo_id = %photo_id,
        file_path = %file_path,
        full_path = %full_path.display(),
        size_bytes = size_bytes,
        "serve_photo: serving file"
    );

    let file = tokio::fs::File::open(&full_path).await.map_err(|e| {
        tracing::error!(
            user_id = %auth.user_id,
            photo_id = %photo_id,
            file_path = %file_path,
            full_path = %full_path.display(),
            error = %e,
            "serve_photo: failed to open file on disk"
        );
        match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open photo: {}", e)),
        }
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            "Content-Type",
            HeaderValue::from_str(&mime_type).unwrap_or(HeaderValue::from_static("application/octet-stream")),
        )
        .header("Content-Length", HeaderValue::from(size_bytes))
        .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"))
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// GET /api/photos/:id/thumb
/// Serve the thumbnail for a plain-mode photo.
pub async fn serve_thumbnail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    let thumb_path: Option<String> = sqlx::query_scalar(
        "SELECT thumb_path FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;
    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&thumb_path);

    // If thumbnail doesn't exist yet, return 202 Accepted to signal "pending"
    // instead of falling back to the original file (which may be a raw format
    // like CR2/HEVC that the browser cannot render).
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Ok(Response::builder()
            .status(StatusCode::ACCEPTED)
            .header("Content-Type", HeaderValue::from_static("application/json"))
            .body(Body::from(r#"{"status":"pending"}"#))
            .map_err(|e| AppError::Internal(e.to_string()))?);
    }

    let meta = tokio::fs::metadata(&full_path).await.map_err(|e| {
        AppError::Internal(format!("Failed to read thumbnail: {}", e))
    })?;
    let file = tokio::fs::File::open(&full_path).await.map_err(|e| {
        AppError::Internal(format!("Failed to open thumbnail: {}", e))
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_static("image/jpeg"))
        .header("Content-Length", HeaderValue::from(meta.len()))
        .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"))
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

// DELETE /api/photos/:id is now handled by trash::handlers::soft_delete_photo
// Photos are soft-deleted to the trash with a 30-day retention period.

/// Returns the web preview file extension if this filename's format is not browser-native.
/// Re-exported from scan module for consistency.
fn needs_web_preview(filename: &str) -> Option<&'static str> {
    super::scan::needs_web_preview(filename)
}

/// GET /api/photos/:id/web
/// Serve a browser-compatible version of the media.
/// If a web preview exists (pre-generated by scan for non-browser-native formats),
/// serve that. Otherwise, serve the original file.
pub async fn serve_web(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    let (file_path, mime_type, filename, size_bytes): (String, String, String, i64) = sqlx::query_as(
        "SELECT file_path, mime_type, filename, size_bytes FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let storage_root = state.storage_root.read().await.clone();

    // Check for a pre-generated web preview (non-browser-native formats)
    if let Some(ext) = needs_web_preview(&filename) {
        let preview_path =
            storage_root.join(format!(".web_previews/{}.web.{}", photo_id, ext));
        if tokio::fs::try_exists(&preview_path).await.unwrap_or(false) {
            let meta = tokio::fs::metadata(&preview_path).await.ok();
            let file = tokio::fs::File::open(&preview_path).await.map_err(|e| {
                AppError::Internal(format!("Failed to open web preview: {}", e))
            })?;
            let content_type = match ext {
                "jpg" => "image/jpeg",
                "png" => "image/png",
                "mp3" => "audio/mpeg",
                "mp4" => "video/mp4",
                _ => "application/octet-stream",
            };
            let stream = tokio_util::io::ReaderStream::new(file);
            let body = Body::from_stream(stream);
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", HeaderValue::from_static(content_type))
                .header(
                    "Cache-Control",
                    HeaderValue::from_static("private, max-age=86400"),
                );
            if let Some(m) = meta {
                builder = builder.header("Content-Length", HeaderValue::from(m.len()));
            }
            return builder
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()));
        } else {
            // Web preview needed but not generated yet — return 202 to signal
            // "conversion in progress" instead of falling back to the raw file
            // (which the browser likely cannot render).
            return Ok(Response::builder()
                .status(StatusCode::ACCEPTED)
                .header("Content-Type", HeaderValue::from_static("application/json"))
                .body(Body::from(r#"{"status":"converting"}"#))
                .map_err(|e| AppError::Internal(e.to_string()))?);
        }
    }

    // Format is browser-native — serve the original file
    let full_path = storage_root.join(&file_path);
    let file = tokio::fs::File::open(&full_path).await.map_err(|e| {
        match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open file: {}", e)),
        }
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            "Content-Type",
            HeaderValue::from_str(&mime_type)
                .unwrap_or(HeaderValue::from_static("application/octet-stream")),
        )
        .header("Content-Length", HeaderValue::from(size_bytes))
        .header(
            "Cache-Control",
            HeaderValue::from_static("private, max-age=86400"),
        )
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

// ── Favorite Toggle ─────────────────────────────────────────────────────────

/// PUT /api/photos/:id/favorite
/// Toggle the is_favorite flag on a photo.
pub async fn toggle_favorite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Toggle: set is_favorite = 1 - is_favorite (0→1, 1→0)
    let rows = sqlx::query(
        "UPDATE photos SET is_favorite = 1 - is_favorite WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Err(AppError::NotFound);
    }

    let is_favorite: bool = sqlx::query_scalar(
        "SELECT is_favorite FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({
        "id": photo_id,
        "is_favorite": is_favorite,
    })))
}

// ── Crop Metadata ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetCropRequest {
    pub crop_metadata: Option<String>,
}

/// PUT /api/photos/:id/crop
/// Set (or clear) crop metadata for a photo.
/// crop_metadata is a JSON string describing the crop rectangle:
/// {"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 0}
/// Values are percentages (0.0-1.0) of original dimensions.
/// Send null to clear the crop.
pub async fn set_crop(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(req): Json<SetCropRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate crop_metadata is valid JSON if provided, and limit size
    if let Some(ref crop) = req.crop_metadata {
        let crop = sanitize::sanitize_freeform(crop, 1024);
        if serde_json::from_str::<serde_json::Value>(&crop).is_err() {
            return Err(AppError::BadRequest("crop_metadata must be valid JSON".into()));
        }
    }

    let rows = sqlx::query(
        "UPDATE photos SET crop_metadata = ? WHERE id = ? AND user_id = ?",
    )
    .bind(req.crop_metadata.as_ref().map(|c| sanitize::sanitize_freeform(c, 1024)))
    .bind(&photo_id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Err(AppError::NotFound);
    }

    // Regenerate thumbnail for plain mode
    let photo: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT file_path, mime_type, thumb_path FROM photos WHERE id = ? AND user_id = ?"
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    if let Some((file_path, mime_type, thumb_path)) = photo {
        if let Some(thumb) = thumb_path {
            let storage_root = state.storage_root.read().await.clone();
            let abs_file = storage_root.join(file_path);
            let abs_thumb = storage_root.join(&thumb);
            let crop_meta = req.crop_metadata.as_deref();
            crate::photos::scan::generate_thumbnail_file(&abs_file, &abs_thumb, &mime_type, crop_meta).await;
        }
    }

    Ok(Json(serde_json::json!({
        "id": photo_id,
        "crop_metadata": req.crop_metadata,
    })))
}
