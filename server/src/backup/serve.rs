use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use super::models::BackupPhotoRecord;

// ── API-Key Validation ───────────────────────────────────────────────────────

/// Validate the `X-API-Key` header against the configured backup API key.
/// Returns an error if the key is missing, wrong, or backup serving is disabled.
fn validate_api_key(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let configured_key = state
        .config
        .backup
        .api_key
        .as_deref()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            AppError::Forbidden("Backup serving is not enabled on this server".into())
        })?;

    let provided_key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing X-API-Key header".into()))?;

    if provided_key != configured_key {
        return Err(AppError::Unauthorized("Invalid API key".into()));
    }

    Ok(())
}

// ── Backup Serve Endpoints ───────────────────────────────────────────────────

/// GET /api/backup/list
/// Returns a list of all photos on this server, authenticated via API key.
/// Used by other servers for recovery and backup browsing.
pub async fn backup_list_photos(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<BackupPhotoRecord>>, AppError> {
    validate_api_key(&state, &headers)?;

    let photos = sqlx::query_as::<_, BackupPhotoRecord>(
        "SELECT p.id, p.filename, p.file_path, p.mime_type, p.media_type, \
         p.size_bytes, p.width, p.height, p.duration_secs, p.taken_at, \
         p.latitude, p.longitude, p.thumb_path, p.created_at \
         FROM photos p ORDER BY p.created_at ASC",
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(photos))
}

/// GET /api/backup/download/:photo_id
/// Serves the original photo file, authenticated via API key.
/// Used by other servers during recovery.
pub async fn backup_download_photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    validate_api_key(&state, &headers)?;

    let (file_path, mime_type): (String, String) = sqlx::query_as(
        "SELECT file_path, mime_type FROM photos WHERE id = ?",
    )
    .bind(&photo_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&file_path);

    let file = tokio::fs::File::open(&full_path).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound,
        _ => AppError::Internal(format!("Failed to open photo: {}", e)),
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_str(&mime_type).unwrap_or(HeaderValue::from_static("application/octet-stream")))
        .body(body)
        .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?;

    // Include the file_path so the caller knows where to store it
    if let Ok(val) = HeaderValue::from_str(&file_path) {
        response.headers_mut().insert("X-File-Path", val);
    }

    Ok(response)
}

/// GET /api/backup/download/:photo_id/thumb
/// Serves the thumbnail for a photo, authenticated via API key.
/// Used by other servers for backup view browsing.
pub async fn backup_download_thumb(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    validate_api_key(&state, &headers)?;

    let thumb_path: Option<String> = sqlx::query_scalar(
        "SELECT thumb_path FROM photos WHERE id = ?",
    )
    .bind(&photo_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;

    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&thumb_path);

    let file = tokio::fs::File::open(&full_path).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound,
        _ => AppError::Internal(format!("Failed to open thumbnail: {}", e)),
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/jpeg")
        .body(body)
        .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?)
}

// ── Backup Receive Endpoint ──────────────────────────────────────────────────

/// POST /api/backup/receive
/// Receives a file from a primary server during sync.
/// Headers: X-API-Key, X-Photo-Id, X-File-Path, X-Source ("photos" or "trash")
/// Body: raw file bytes
pub async fn backup_receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_api_key(&state, &headers)?;

    // Extract required headers
    let photo_id = headers
        .get("X-Photo-Id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-Photo-Id header".into()))?
        .to_string();

    let file_path = headers
        .get("X-File-Path")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-File-Path header".into()))?
        .to_string();

    // Security: validate the file_path is a safe relative path (no traversal, no absolute)
    sanitize::validate_relative_path(&file_path)
        .map_err(|reason| AppError::BadRequest(format!("Invalid X-File-Path: {}", reason)))?;

    let source = headers
        .get("X-Source")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("photos")
        .to_string();

    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&file_path);

    // Defense-in-depth: verify the resolved path is still within storage_root
    let canonical_root = storage_root.canonicalize().unwrap_or_else(|_| storage_root.clone());
    // We can't canonicalize full_path yet (file doesn't exist), so check the parent
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            AppError::Internal(format!("Failed to create directories: {}", e))
        })?;
        let canonical_parent = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());
        if !canonical_parent.starts_with(&canonical_root) {
            return Err(AppError::BadRequest("File path escapes storage root".into()));
        }
    }

    // Write the file to disk
    let size_bytes = body.len() as i64;
    tokio::fs::write(&full_path, &body).await.map_err(|e| {
        AppError::Internal(format!("Failed to write file: {}", e))
    })?;

    // Get (or create) a user to own the synced photo — use first admin user
    let admin_id: String = sqlx::query_scalar(
        "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Internal("No admin user on backup server".into()))?;

    // Derive filename and mime_type from file_path
    let filename = std::path::Path::new(&file_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.clone());

    let mime_type = match std::path::Path::new(&file_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") | Some("heif") => "image/heic",
        Some("bmp") => "image/bmp",
        Some("tiff") | Some("tif") => "image/tiff",
        Some("svg") => "image/svg+xml",
        Some("ico") | Some("cur") => "image/x-icon",
        Some("hdr") => "image/vnd.radiance",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("avi") => "video/x-msvideo",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("wmv") => "video/x-ms-wmv",
        Some("asf") => "video/x-ms-asf",
        Some("hevc") | Some("h265") => "video/hevc",
        Some("h264") => "video/h264",
        Some("mpg") | Some("mpeg") => "video/mpeg",
        Some("m4v") => "video/x-m4v",
        Some("mp3") => "audio/mpeg",
        Some("aiff") => "audio/aiff",
        Some("flac") => "audio/flac",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("wma") => "audio/x-ms-wma",
        _ => "application/octet-stream",
    }
    .to_string();

    let media_type = if mime_type.starts_with("video/") {
        "video"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else if mime_type == "image/gif" {
        "gif"
    } else {
        "photo"
    };

    let now = Utc::now().to_rfc3339();

    if source == "trash" {
        // Upsert into trash_items
        sqlx::query(
            "INSERT INTO trash_items (id, user_id, photo_id, filename, file_path, mime_type, \
             media_type, size_bytes, deleted_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET file_path = excluded.file_path, \
             size_bytes = excluded.size_bytes",
        )
        .bind(&photo_id)
        .bind(&admin_id)
        .bind(&photo_id) // photo_id == original photo ID for trash
        .bind(&filename)
        .bind(&file_path)
        .bind(&mime_type)
        .bind(media_type)
        .bind(size_bytes)
        .bind(&now)
        .bind(&now) // expires_at — not important for backups, just needs a value
        .execute(&state.pool)
        .await?;
    } else {
        // Upsert into photos
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET file_path = excluded.file_path, \
             size_bytes = excluded.size_bytes",
        )
        .bind(&photo_id)
        .bind(&admin_id)
        .bind(&filename)
        .bind(&file_path)
        .bind(&mime_type)
        .bind(media_type)
        .bind(size_bytes)
        .bind(&now)
        .execute(&state.pool)
        .await?;
    }

    tracing::debug!("Received backup {} ({} bytes): {}", source, size_bytes, file_path);

    Ok(Json(serde_json::json!({
        "status": "ok",
        "photo_id": photo_id,
        "size_bytes": size_bytes,
    })))
}
