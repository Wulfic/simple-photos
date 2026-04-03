//! Server‐to‐server backup “serving” API.
//!
//! When this instance is in backup mode, other Simple Photos servers can
//! pull data from it via these endpoints. All requests are authenticated
//! with an `X-API-Key` header (validated against `config.backup.api_key`).
//!
//! Endpoints:
//! - `GET  /api/backup/list`                    — list all photos (with IDs)
//! - `GET  /api/backup/list-trash`              — list all trash items (with IDs)
//! - `GET  /api/backup/download/:id`            — download original file
//! - `GET  /api/backup/download/:id/thumb`      — download thumbnail
//! - `POST /api/backup/receive`                 — receive a photo pushed
//!   from the primary server (verifies `X-Content-Hash` if present)

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use percent_encoding::{utf8_percent_encode, CONTROLS};

use crate::error::AppError;
use crate::state::AppState;

use super::models::BackupPhotoRecord;

// ── API-Key Validation ───────────────────────────────────────────────────────

/// Validate the `X-API-Key` header against the configured backup API key.
///
/// Priority:
/// 1. `config.backup.api_key` (TOML / env var) — static, fastest path
/// 2. `server_settings.backup_api_key` (DB) — auto-generated during pairing
///    or when "backup mode" is enabled via the admin UI
///
/// Returns an error if the key is missing, wrong, or backup serving is disabled.
pub(super) async fn validate_api_key(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    // Resolve the expected key: prefer static config, fall back to DB
    let configured_key: String = if let Some(k) = state
        .config
        .backup
        .api_key
        .as_deref()
        .filter(|k| !k.is_empty())
    {
        k.to_string()
    } else {
        // Check DB for a key generated via pairing or admin UI
        let db_key: Option<String> =
            sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_api_key'")
                .fetch_optional(&state.read_pool)
                .await
                .unwrap_or(None);

        db_key.filter(|k| !k.is_empty()).ok_or_else(|| {
            AppError::Forbidden("Backup serving is not enabled on this server".into())
        })?
    };

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
    validate_api_key(&state, &headers).await?;

    let photos = sqlx::query_as::<_, BackupPhotoRecord>(
        "SELECT p.id, p.user_id, p.filename, p.file_path, p.mime_type, p.media_type, \
         p.size_bytes, p.width, p.height, p.duration_secs, p.taken_at, \
         p.latitude, p.longitude, p.thumb_path, p.created_at, \
         p.is_favorite, p.camera_model, p.photo_hash, p.crop_metadata \
         FROM photos p ORDER BY p.created_at ASC",
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
            _ => AppError::Internal(format!("Failed to open photo: {}", e)),
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
        .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?;

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

// ── Deletion Sync Endpoint ──────────────────────────────────────────────────

/// POST /api/backup/sync-deletions
/// Accepts a list of photo IDs that have been deleted on the primary server
/// (i.e., are now in the primary's trash) and removes them from the backup's
/// `photos` + `photo_tags` tables so the items no longer appear in the gallery.
///
/// This is called during every sync so that items deleted on the primary are
/// evicted from the backup gallery even when they were already in
/// `remote_trash_ids` (and therefore skipped by the file-transfer delta logic).
pub async fn backup_sync_deletions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let ids: Vec<String> = body
        .get("deleted_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if ids.is_empty() {
        return Ok(StatusCode::OK);
    }

    let mut removed = 0usize;
    for id in &ids {
        // Remove from gallery; ignore if it wasn't there (nothing to do).
        let result = sqlx::query("DELETE FROM photos WHERE id = ?")
            .bind(id)
            .execute(&state.pool)
            .await;
        match result {
            Ok(r) if r.rows_affected() > 0 => {
                removed += 1;
                // Clean up orphaned tags for the removed row.
                let _ = sqlx::query("DELETE FROM photo_tags WHERE photo_id = ?")
                    .bind(id)
                    .execute(&state.pool)
                    .await;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(photo_id = %id, "sync-deletions: failed to remove from photos: {}", e);
            }
        }
    }

    if removed > 0 {
        tracing::info!(
            "sync-deletions: removed {} photo(s) from gallery that are now in primary trash",
            removed
        );
    }

    Ok(StatusCode::OK)
}

