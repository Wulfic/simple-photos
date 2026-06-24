//! Read-only backup serve endpoints: photo/trash/blob listing and file
//! download. All authenticated via `X-API-Key`.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use percent_encoding::{utf8_percent_encode, CONTROLS};

use crate::error::AppError;
use crate::state::AppState;

use super::super::models::BackupPhotoRecord;
use super::validate_api_key;

// ── Backup Serve Endpoints ───────────────────────────────────────────────────

/// GET /api/backup/list
/// Returns a list of all photos on this server, authenticated via API key.
/// Used by other servers for recovery and backup browsing.
pub async fn backup_list_photos(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<BackupPhotoRecord>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let photos = sqlx::query_as::<_, BackupPhotoRecord>(
        "SELECT p.id, p.user_id, p.filename, p.file_path, p.mime_type, p.media_type, \
         p.size_bytes, p.width, p.height, p.duration_secs, p.taken_at, \
         p.latitude, p.longitude, p.thumb_path, p.created_at, \
         p.is_favorite, p.camera_model, p.photo_hash, p.crop_metadata \
         FROM photos p \
         WHERE p.id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
           AND p.id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
         ORDER BY p.created_at ASC",
    )
    .fetch_all(&state.read_pool)
    .await?;

    Ok(Json(photos))
}

/// GET /api/backup/list-trash
/// Returns a list of all trash items on this server, authenticated via API key.
/// Used by the sync engine for delta-sync (skip items the remote already has).
pub async fn backup_list_trash(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let rows: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT id, file_path, size_bytes FROM trash_items ORDER BY deleted_at ASC")
            .fetch_all(&state.read_pool)
            .await?;

    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, file_path, size_bytes)| {
            serde_json::json!({ "id": id, "file_path": file_path, "size_bytes": size_bytes })
        })
        .collect();

    Ok(Json(items))
}

/// GET /api/backup/download/:photo_id
/// Serves the original photo file, authenticated via API key.
/// Used by other servers during recovery.
pub async fn backup_download_photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    validate_api_key(&state, &headers).await?;

    let (file_path, mime_type): (String, String) =
        sqlx::query_as("SELECT file_path, mime_type FROM photos WHERE id = ?")
            .bind(&photo_id)
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&file_path);

    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open photo: {e}")),
        })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(
            "Content-Type",
            HeaderValue::from_str(&mime_type)
                .unwrap_or(HeaderValue::from_static("application/octet-stream")),
        )
        .body(body)
        .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?;

    // Include the file_path so the caller knows where to store it.
    // Percent-encode non-ASCII chars so the header value stays valid ASCII.
    let encoded_fp = utf8_percent_encode(&file_path, CONTROLS).to_string();
    if let Ok(val) = HeaderValue::from_str(&encoded_fp) {
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
    validate_api_key(&state, &headers).await?;

    let thumb_path: Option<String> =
        sqlx::query_scalar("SELECT thumb_path FROM photos WHERE id = ?")
            .bind(&photo_id)
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;

    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&thumb_path);

    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open thumbnail: {e}")),
        })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/jpeg")
        .body(body)
        .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
}

// ── Blob List Endpoint ───────────────────────────────────────────────────────

/// GET /api/backup/list-blobs
/// Returns a list of all blob IDs on this server for delta sync.
pub async fn backup_list_blobs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let blobs: Vec<(String, i64)> = sqlx::query_as(
        "SELECT id, size_bytes FROM blobs \
         WHERE blob_type != 'gallery-placeholder' \
           AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
           AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
           AND id NOT IN ( \
               SELECT p.encrypted_blob_id FROM photos p \
               WHERE p.encrypted_blob_id IS NOT NULL \
               AND p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
               AND p.encrypted_blob_id NOT IN ( \
                   SELECT p2.encrypted_blob_id FROM photos p2 \
                   WHERE p2.encrypted_blob_id IS NOT NULL \
                   AND p2.id IN (SELECT blob_id FROM encrypted_gallery_items)) \
               AND p.encrypted_blob_id NOT IN ( \
                   SELECT egi.encrypted_blob_id FROM encrypted_gallery_items egi \
                   WHERE egi.encrypted_blob_id IS NOT NULL)) \
           AND id NOT IN ( \
               SELECT p.encrypted_thumb_blob_id FROM photos p \
               WHERE p.encrypted_thumb_blob_id IS NOT NULL \
               AND p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
               AND p.encrypted_thumb_blob_id NOT IN ( \
                   SELECT p2.encrypted_thumb_blob_id FROM photos p2 \
                   WHERE p2.encrypted_thumb_blob_id IS NOT NULL \
                   AND p2.id IN (SELECT blob_id FROM encrypted_gallery_items)) \
               AND p.encrypted_thumb_blob_id NOT IN ( \
                   SELECT egi.encrypted_thumb_blob_id FROM encrypted_gallery_items egi \
                   WHERE egi.encrypted_thumb_blob_id IS NOT NULL)) \
           AND id NOT IN (SELECT encrypted_blob_id FROM encrypted_gallery_items WHERE encrypted_blob_id IS NOT NULL) \
           AND id NOT IN (SELECT encrypted_thumb_blob_id FROM encrypted_gallery_items WHERE encrypted_thumb_blob_id IS NOT NULL) \
         ORDER BY upload_time ASC",
    )
    .fetch_all(&state.read_pool)
    .await?;

    let result: Vec<serde_json::Value> = blobs
        .iter()
        .map(|(id, size)| serde_json::json!({ "id": id, "size_bytes": size }))
        .collect();

    Ok(Json(result))
}
