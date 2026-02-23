use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;

use crate::error::AppError;
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
