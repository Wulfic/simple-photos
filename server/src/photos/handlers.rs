//! Photo management endpoints (photos table — used by autoscan and conversion pipeline).
//!
//! Covers listing (paginated, sorted by `taken_at`), registration from
//! on-disk files, serving originals / thumbnails / web-previews,
//! favorite toggling, and crop-metadata storage.
//!
//! `serve_photo` and `serve_web` support HTTP Range requests for video
//! seeking and large-file download resumption.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use super::models::*;
use super::utils::{normalize_iso_timestamp, utc_now_iso};

// ── Plain Photo Endpoints ─────────────────────────────────────────────────────

/// Query parameters for `GET /api/photos`.
#[derive(Debug, Deserialize)]
pub struct PhotoListQuery {
    /// Cursor for reverse-chronological pagination (taken_at or created_at).
    pub after: Option<String>,
    /// Maximum items to return (default 100, max 500).
    pub limit: Option<i64>,
    /// Filter by media type: "photo", "video", "gif", "audio".
    pub media_type: Option<String>,
    /// When `true`, return only favorited photos.
    pub favorites_only: Option<bool>,
}

/// GET /api/photos
/// List photos in the photos table for the authenticated user.
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
         FROM photos WHERE user_id = ?"
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

    let photos = query.fetch_all(&state.read_pool).await?;

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
/// Register a file on disk as a photo in the database.
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

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
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

    // Compute content-based hash using streaming I/O (64 KB chunks) so large
    // files never need to be buffered entirely in memory.
    let photo_hash = super::utils::compute_photo_hash_streaming(&full_path)
        .await
        .unwrap_or_default();

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

/// Stream buffer size for file serving — 64 KB per chunk instead of the
/// default 4 KB.  Larger chunks reduce the number of syscalls and context
/// switches, which is critical when serving large video files or many
/// thumbnails concurrently.
const STREAM_BUF_SIZE: usize = 64 * 1024;

/// Check `If-None-Match` header against our ETag.  Returns `Some(304)` if
/// the client already has the current version.
fn check_etag(headers: &HeaderMap, etag: &str) -> Option<Response> {
    if let Some(inm) = headers.get("if-none-match").and_then(|v| v.to_str().ok()) {
        if inm == etag || inm.trim_matches('"') == etag.trim_matches('"') {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header("ETag", HeaderValue::from_str(etag).unwrap_or(HeaderValue::from_static("")))
                .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"))
                .body(Body::empty())
                .ok();
        }
    }
    None
}

/// GET /api/photos/:id/file
/// Serve the **original** (unconverted) photo/video/audio file from disk.
/// This always returns the original format — even after the background
/// pipeline has generated a web-compatible copy in `.web_previews/`.
/// Supports HTTP Range requests for video seeking and download resumption.
/// Returns ETag for caching; responds with 304 Not Modified on cache hit.
pub async fn serve_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    let (file_path, mime_type, size_bytes): (String, String, i64) = sqlx::query_as(
        "SELECT file_path, mime_type, size_bytes FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or_else(|| {
        tracing::warn!(
            user_id = %auth.user_id,
            photo_id = %photo_id,
            "serve_photo: photo not found in database"
        );
        AppError::NotFound
    })?;

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&file_path);

    tracing::debug!(
        user_id = %auth.user_id,
        photo_id = %photo_id,
        file_path = %file_path,
        full_path = %full_path.display(),
        size_bytes = size_bytes,
        "serve_photo: serving file"
    );

    let total_size = size_bytes as u64;
    let content_type = HeaderValue::from_str(&mime_type)
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));

    let open_file = || async {
        tokio::fs::File::open(&full_path).await.map_err(|e| {
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
        })
    };

    // ── ETag / conditional response ─────────────────────────────────────
    let etag = format!("\"{}-{}\"", photo_id, total_size);
    if let Some(not_modified) = check_etag(&headers, &etag) {
        return Ok(not_modified);
    }

    // ── HTTP Range support ─────────────────────────────────────────────
    if let Some(range_header) = headers.get("range").and_then(|v| v.to_str().ok()) {
        if let Some((start, end)) = crate::blobs::handlers::parse_range_header(range_header, total_size) {
            let length = end - start + 1;
            let mut file = open_file().await?;

            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| AppError::Internal(format!("Failed to seek: {}", e)))?;

            let stream = tokio_util::io::ReaderStream::with_capacity(file.take(length), STREAM_BUF_SIZE);
            let body = Body::from_stream(stream);

            return Ok(Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Type", content_type)
                .header("Content-Length", HeaderValue::from(length))
                .header("Content-Range", HeaderValue::from_str(
                    &format!("bytes {}-{}/{}", start, end, total_size)
                ).map_err(|e| AppError::Internal(format!("Invalid header: {}", e)))?)
                .header("Accept-Ranges", HeaderValue::from_static("bytes"))
                .header("ETag", HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")))
                .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"))
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()))?);
        } else {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("Content-Range", HeaderValue::from_str(
                    &format!("bytes */{}", total_size)
                ).map_err(|e| AppError::Internal(format!("Invalid header: {}", e)))?)
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()))?);
        }
    }

    // ── Full download ──────────────────────────────────────────────────
    let file = open_file().await?;
    let stream = tokio_util::io::ReaderStream::with_capacity(file, STREAM_BUF_SIZE);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", content_type)
        .header("Content-Length", HeaderValue::from(size_bytes))
        .header("Accept-Ranges", HeaderValue::from_static("bytes"))
        .header("ETag", HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")))
        .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"))
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// GET /api/photos/:id/thumb
/// Serve the thumbnail for a photo.
/// Returns ETag for caching; responds with 304 Not Modified on cache hit.
pub async fn serve_thumbnail(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    let thumb_path: Option<String> = sqlx::query_scalar(
        "SELECT thumb_path FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;
    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
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

    // ETag for thumbnails — ID + file size on disk
    let etag = format!("\"{}-thumb-{}\"", photo_id, meta.len());
    if let Some(not_modified) = check_etag(&headers, &etag) {
        return Ok(not_modified);
    }

    let file = tokio::fs::File::open(&full_path).await.map_err(|e| {
        AppError::Internal(format!("Failed to open thumbnail: {}", e))
    })?;

    let stream = tokio_util::io::ReaderStream::with_capacity(file, STREAM_BUF_SIZE);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_static("image/jpeg"))
        .header("Content-Length", HeaderValue::from(meta.len()))
        .header("ETag", HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")))
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
/// Serve a browser/Android-compatible version of the media.
///
/// If the format needs conversion (HEIC→JPEG, MKV→MP4, etc.) and the
/// background pipeline has already generated a web preview in
/// `.web_previews/`, serve the converted copy.  Otherwise, for
/// browser-native formats, serve the original file directly.
///
/// Returns 202 "converting" if the web preview is needed but hasn't been
/// generated yet (the pipeline will create it on its next cycle).
///
/// The original file is always preserved — use `/photos/:id/file` to
/// download it in its original format.
///
/// Supports HTTP Range requests for video seeking.
pub async fn serve_web(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    let (file_path, mime_type, filename, size_bytes): (String, String, String, i64) = sqlx::query_as(
        "SELECT file_path, mime_type, filename, size_bytes FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let storage_root = (**state.storage_root.load()).clone();

    // Check for a pre-generated web preview (non-browser-native formats)
    if let Some(ext) = needs_web_preview(&filename) {
        let preview_path =
            storage_root.join(format!(".web_previews/{}.web.{}", photo_id, ext));
        if tokio::fs::try_exists(&preview_path).await.unwrap_or(false) {
            let meta = tokio::fs::metadata(&preview_path).await.map_err(|e| {
                AppError::Internal(format!("Failed to read web preview metadata: {}", e))
            })?;
            let total_size = meta.len();
            let content_type: &str = match ext {
                "jpg" => "image/jpeg",
                "png" => "image/png",
                "mp3" => "audio/mpeg",
                "mp4" => "video/mp4",
                _ => "application/octet-stream",
            };

            let etag = format!("\"{}-web-{}\"", photo_id, total_size);
            return serve_file_with_range(
                &preview_path, total_size, content_type, &headers, Some(&etag),
            ).await;
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
    let content_type = mime_type.as_str();

    let etag = format!("\"{}-orig-{}\"", photo_id, size_bytes);
    serve_file_with_range(&full_path, size_bytes as u64, content_type, &headers, Some(&etag)).await
}

/// Internal helper: serve a file with optional HTTP Range + ETag support.
/// `etag` is optional — if provided, the response includes the ETag header
/// and If-None-Match is checked for 304 early-return.
async fn serve_file_with_range(
    path: &std::path::Path,
    total_size: u64,
    content_type: &str,
    headers: &HeaderMap,
    etag: Option<&str>,
) -> Result<Response, AppError> {
    let ct = HeaderValue::from_str(content_type)
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));

    // ETag conditional check
    if let Some(tag) = etag {
        if let Some(not_modified) = check_etag(headers, tag) {
            return Ok(not_modified);
        }
    }

    let etag_hv = etag.and_then(|t| HeaderValue::from_str(t).ok());

    if let Some(range_header) = headers.get("range").and_then(|v| v.to_str().ok()) {
        if let Some((start, end)) = crate::blobs::handlers::parse_range_header(range_header, total_size) {
            let length = end - start + 1;
            let mut file = tokio::fs::File::open(path).await.map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => AppError::NotFound,
                _ => AppError::Internal(format!("Failed to open file: {}", e)),
            })?;

            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| AppError::Internal(format!("Failed to seek: {}", e)))?;

            let stream = tokio_util::io::ReaderStream::with_capacity(file.take(length), STREAM_BUF_SIZE);
            let body = Body::from_stream(stream);

            let mut builder = Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Type", ct)
                .header("Content-Length", HeaderValue::from(length))
                .header("Content-Range", HeaderValue::from_str(
                    &format!("bytes {}-{}/{}", start, end, total_size)
                ).map_err(|e| AppError::Internal(format!("Invalid header: {}", e)))?)
                .header("Accept-Ranges", HeaderValue::from_static("bytes"))
                .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"));
            if let Some(ref ev) = etag_hv {
                builder = builder.header("ETag", ev.clone());
            }
            return Ok(builder.body(body)
                .map_err(|e| AppError::Internal(e.to_string()))?);
        } else {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("Content-Range", HeaderValue::from_str(
                    &format!("bytes */{}", total_size)
                ).map_err(|e| AppError::Internal(format!("Invalid header: {}", e)))?)
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()))?);
        }
    }

    let file = tokio::fs::File::open(path).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound,
        _ => AppError::Internal(format!("Failed to open file: {}", e)),
    })?;

    let stream = tokio_util::io::ReaderStream::with_capacity(file, STREAM_BUF_SIZE);
    let body = Body::from_stream(stream);

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", ct)
        .header("Content-Length", HeaderValue::from(total_size))
        .header("Accept-Ranges", HeaderValue::from_static("bytes"))
        .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"));
    if let Some(ref ev) = etag_hv {
        builder = builder.header("ETag", ev.clone());
    }
    Ok(builder.body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

// ── Favorite Toggle ─────────────────────────────────────────────────────────

/// PUT /api/photos/:id/favorite
/// Toggle the is_favorite flag on a photo.
///
/// **Performance:** Uses `RETURNING` (SQLite 3.35+) to get the new value in
/// the same statement, eliminating a second SELECT round-trip.
pub async fn toggle_favorite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Toggle and return new value in a single statement (RETURNING, SQLite 3.35+).
    // Eliminates a second SELECT query that was previously needed to read back
    // the toggled value.
    let new_fav: Option<bool> = sqlx::query_scalar(
        "UPDATE photos SET is_favorite = 1 - is_favorite \
         WHERE id = ? AND user_id = ? \
         RETURNING is_favorite",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let is_favorite = new_fav.ok_or(AppError::NotFound)?;

    Ok(Json(serde_json::json!({
        "id": photo_id,
        "is_favorite": is_favorite,
    })))
}

// ── Crop Metadata ───────────────────────────────────────────────────────────

/// Request body for `PUT /api/photos/{id}/crop`.
/// `crop_metadata` is a JSON string describing the crop rectangle as percentage
/// coordinates: `{"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 0}`.
/// Send `null` to clear the crop.
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

    Ok(Json(serde_json::json!({
        "id": photo_id,
        "crop_metadata": req.crop_metadata,
    })))
}
