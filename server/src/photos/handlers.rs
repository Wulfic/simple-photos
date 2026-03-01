use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{SecondsFormat, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::mime_from_extension;
use crate::state::AppState;

use super::metadata::extract_media_metadata_from_bytes;
use super::models::*;

/// Produce a UTC ISO-8601 timestamp with millisecond precision and Z suffix.
/// Format: `2024-02-28T22:44:29.043Z`
///
/// This is critical for consistent text-based sorting in SQLite — all
/// timestamps (taken_at, created_at) must use the same format so that
/// `ORDER BY COALESCE(taken_at, created_at) DESC` works correctly.
pub fn utc_now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Normalize a timestamp string to consistent ISO-8601 Z-suffix format.
/// Handles:
/// - Naive "2024-01-15T14:30:00" → "2024-01-15T14:30:00.000Z" (treated as UTC)
/// - Offset "+00:00" → "Z"
/// - Already "Z" → passed through
pub fn normalize_iso_timestamp(ts: &str) -> String {
    // Try parsing as a full DateTime<Utc> or DateTime<FixedOffset>
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        return dt.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true);
    }
    // Try parsing as naive datetime (no timezone) — treat as UTC
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.f") {
        let dt = naive.and_utc();
        return dt.to_rfc3339_opts(SecondsFormat::Millis, true);
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S") {
        let dt = naive.and_utc();
        return dt.to_rfc3339_opts(SecondsFormat::Millis, true);
    }
    // Fallback: return as-is
    ts.to_string()
}

/// Compute a short content-based hash: first 12 hex chars of SHA-256.
/// This deterministic fingerprint is the same regardless of which platform
/// uploads the photo, guaranteeing cross-platform alignment.
pub fn compute_photo_hash(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex::encode(&digest[..6]) // 6 bytes → 12 hex chars (48-bit)
}

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

    sql.push_str(" ORDER BY COALESCE(taken_at, created_at) DESC LIMIT ?");
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
    // Security: ensure file_path doesn't escape storage root
    if req.file_path.contains("..") {
        return Err(AppError::BadRequest(
            "file_path must not contain '..'".into(),
        ));
    }

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

    // If thumbnail doesn't exist yet, try to generate it on-the-fly
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        // Fall back to serving the original photo (client can resize)
        let (file_path, mime_type): (String, String) = sqlx::query_as(
            "SELECT file_path, mime_type FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&photo_id)
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;

        let orig_path = storage_root.join(&file_path);
        let file = tokio::fs::File::open(&orig_path).await.map_err(|e| {
            AppError::Internal(format!("Failed to open photo for thumbnail: {}", e))
        })?;

        let stream = tokio_util::io::ReaderStream::new(file);
        let body = Body::from_stream(stream);

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(
                "Content-Type",
                HeaderValue::from_str(&mime_type)
                    .unwrap_or(HeaderValue::from_static("image/jpeg")),
            )
            .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"))
            .body(body)
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

/// POST /api/photos/upload
/// Upload a plain photo/video/GIF file from a mobile client.
/// The file body is sent as raw bytes with metadata in headers:
///   X-Filename: original filename
///   X-Mime-Type: MIME type (e.g., image/jpeg)
///   Content-Length: file size in bytes
///
/// The server stores the file in the storage root and registers it as a plain photo.
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
    } else if mime_type == "image/gif" {
        "gif"
    } else {
        "photo"
    };

    let size_bytes = body.len() as i64;

    // Sanitize filename — strip path separators and traversal
    let safe_filename = filename
        .replace(['/', '\\'], "")
        .replace("..", "")
        .trim()
        .to_string();
    let safe_filename = if safe_filename.is_empty() {
        format!("{}.jpg", Uuid::new_v4())
    } else {
        safe_filename
    };

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
    .fetch_optional(&state.pool)
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
    let storage_root = state.storage_root.read().await.clone();
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

    // Extract metadata from the uploaded bytes
    let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
        extract_media_metadata_from_bytes(&body, &final_filename);

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
    let rows = sqlx::query(
        "UPDATE photos SET crop_metadata = ? WHERE id = ? AND user_id = ?",
    )
    .bind(&req.crop_metadata)
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
