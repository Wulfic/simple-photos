//! Trash (soft-delete) management.
//!
//! Photos and encrypted blobs are soft-deleted into `trash_items` with a
//! configurable retention period (default 30 days, stored in
//! `server_settings.trash_retention_days`). Expired items are purged by
//! a background task that runs hourly.
//!
//! Endpoints: list trash, restore, permanent-delete, empty-all,
//! serve trash thumbnail, soft-delete blob (encrypted mode).
//!
//! All multi-step DB operations (INSERT→DELETE, SELECT→DELETE) are wrapped
//! in SQLite transactions for atomicity.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use super::models::*;

// ── Trash Endpoints ───────────────────────────────────────────────────────────

/// GET /api/trash
/// List all items in the authenticated user's trash.
pub async fn list_trash(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<TrashListQuery>,
) -> Result<Json<TrashListResponse>, AppError> {
    let limit = params.limit.unwrap_or(100).min(500);

    let items = if let Some(ref after) = params.after {
        sqlx::query_as::<_, TrashItem>(
            "SELECT id, photo_id, filename, file_path, mime_type, media_type, size_bytes, \
             width, height, duration_secs, taken_at, latitude, longitude, thumb_path, \
             deleted_at, expires_at, encrypted_blob_id, thumbnail_blob_id \
             FROM trash_items WHERE user_id = ? AND deleted_at < ? \
             ORDER BY deleted_at DESC LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(after)
        .bind(limit + 1)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as::<_, TrashItem>(
            "SELECT id, photo_id, filename, file_path, mime_type, media_type, size_bytes, \
             width, height, duration_secs, taken_at, latitude, longitude, thumb_path, \
             deleted_at, expires_at, encrypted_blob_id, thumbnail_blob_id \
             FROM trash_items WHERE user_id = ? \
             ORDER BY deleted_at DESC LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(limit + 1)
        .fetch_all(&state.pool)
        .await?
    };

    let next_cursor = if items.len() as i64 > limit {
        items.last().map(|i| i.deleted_at.clone())
    } else {
        None
    };

    let items: Vec<TrashItem> = items.into_iter().take(limit as usize).collect();

    Ok(Json(TrashListResponse { items, next_cursor }))
}

/// DELETE /api/photos/:id  (updated — now soft-deletes to trash)
/// Move a photo to the trash. The photo row is removed from `photos` and
/// inserted into `trash_items` with a configurable expiration (default 30 days,
/// read from `server_settings.trash_retention_days`).
pub async fn soft_delete_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Begin transaction — INSERT into trash + DELETE from photos must be atomic
    let mut tx = state.pool.begin().await?;

    // Fetch the photo record first
    let photo = sqlx::query_as::<_, TrashPhotoRow>(
        "SELECT id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    // Read retention days from server_settings (default 30)
    let retention_days: i64 = sqlx::query_scalar(
        "SELECT CAST(value AS INTEGER) FROM server_settings WHERE key = 'trash_retention_days'",
    )
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(30);

    let now = Utc::now();
    let expires_at = now + Duration::days(retention_days);
    let trash_id = Uuid::new_v4().to_string();

    // Insert into trash
    sqlx::query(
        "INSERT INTO trash_items (id, user_id, photo_id, filename, file_path, mime_type, \
         media_type, size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
         thumb_path, deleted_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .bind(&photo.id)
    .bind(&photo.filename)
    .bind(&photo.file_path)
    .bind(&photo.mime_type)
    .bind(&photo.media_type)
    .bind(photo.size_bytes)
    .bind(photo.width)
    .bind(photo.height)
    .bind(photo.duration_secs)
    .bind(&photo.taken_at)
    .bind(photo.latitude)
    .bind(photo.longitude)
    .bind(&photo.thumb_path)
    .bind(now.to_rfc3339())
    .bind(expires_at.to_rfc3339())
    .execute(&mut *tx)
    .await?;

    // Remove from photos table
    sqlx::query("DELETE FROM photos WHERE id = ? AND user_id = ?")
        .bind(&photo_id)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    tracing::info!(
        "Photo {} moved to trash (expires {})",
        photo_id,
        expires_at.to_rfc3339()
    );

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/blobs/:id/trash
/// Soft-delete an encrypted blob to the trash. The client provides the metadata
/// since the server stores blobs opaquely.
pub async fn soft_delete_blob(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(blob_id): Path<String>,
    Json(req): Json<SoftDeleteBlobRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate blob_id format
    if Uuid::parse_str(&blob_id).is_err() {
        return Err(AppError::BadRequest("Invalid blob ID format".into()));
    }

    // Sanitize client-supplied metadata before starting the transaction
    let safe_filename = sanitize::sanitize_filename(&req.filename);
    let safe_mime = sanitize::sanitize_freeform(&req.mime_type, 128);
    let media_type = req.media_type.as_deref().unwrap_or("photo");
    let size_bytes = req.size_bytes.unwrap_or(0);

    // Begin transaction — INSERT trash + DELETE blob(s) must be atomic
    let mut tx = state.pool.begin().await?;

    // Fetch the blob record (need storage_path)
    let storage_path = sqlx::query_scalar::<_, String>(
        "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    // Optionally fetch thumbnail blob storage_path
    let thumb_storage_path = if let Some(ref thumb_id) = req.thumbnail_blob_id {
        sqlx::query_scalar::<_, String>(
            "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
        )
        .bind(thumb_id)
        .bind(&auth.user_id)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        None
    };

    // Read retention days from server_settings (default 30)
    let retention_days: i64 = sqlx::query_scalar(
        "SELECT CAST(value AS INTEGER) FROM server_settings WHERE key = 'trash_retention_days'",
    )
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(30);

    let now = Utc::now();
    let expires_at = now + Duration::days(retention_days);
    let trash_id = Uuid::new_v4().to_string();

    // Insert into trash_items with blob references
    sqlx::query(
        "INSERT INTO trash_items (id, user_id, photo_id, filename, file_path, mime_type, \
         media_type, size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
         thumb_path, deleted_at, expires_at, encrypted_blob_id, thumbnail_blob_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?)",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .bind(&blob_id) // photo_id = blob_id for encrypted items
    .bind(&safe_filename)
    .bind(&storage_path) // file_path = blob storage_path
    .bind(&safe_mime)
    .bind(media_type)
    .bind(size_bytes)
    .bind(req.width.unwrap_or(0))
    .bind(req.height.unwrap_or(0))
    .bind(req.duration_secs)
    .bind(&req.taken_at)
    .bind(&thumb_storage_path) // thumb_path = thumbnail blob storage_path
    .bind(now.to_rfc3339())
    .bind(expires_at.to_rfc3339())
    .bind(&blob_id)
    .bind(&req.thumbnail_blob_id)
    .execute(&mut *tx)
    .await?;

    // Remove blob from blobs table (but keep files on disk!)
    sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
        .bind(&blob_id)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    // Also remove thumbnail blob record if present
    if let Some(ref thumb_id) = req.thumbnail_blob_id {
        sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
            .bind(thumb_id)
            .bind(&auth.user_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    tracing::info!(
        "Encrypted blob {} moved to trash (expires {})",
        blob_id,
        expires_at.to_rfc3339()
    );

    Ok(Json(serde_json::json!({
        "trash_id": trash_id,
        "expires_at": expires_at.to_rfc3339(),
    })))
}

/// POST /api/trash/:id/restore
/// Restore a photo from the trash back to the photos table (plain)
/// or the blobs table (encrypted).
pub async fn restore_from_trash(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(trash_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Begin transaction — all restore operations must be atomic
    let mut tx = state.pool.begin().await?;

    // Fetch the trash item to determine if it's a plain photo or encrypted blob
    let encrypted_blob_id: Option<String> = sqlx::query_scalar(
        "SELECT encrypted_blob_id FROM trash_items WHERE id = ? AND user_id = ?",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    if let Some(blob_id) = encrypted_blob_id {
        // ── Encrypted blob restore ──────────────────────────────────────────
        let row = sqlx::query_as::<_, TrashBlobRow>(
            "SELECT file_path, mime_type, media_type, size_bytes, thumb_path, \
             thumbnail_blob_id \
             FROM trash_items WHERE id = ? AND user_id = ?",
        )
        .bind(&trash_id)
        .bind(&auth.user_id)
        .fetch_one(&mut *tx)
        .await?;

        // Determine blob_type from media_type
        let blob_type = match row.media_type.as_str() {
            "gif" => "gif",
            "video" => "video",
            "audio" => "audio",
            _ => "photo",
        };

        let now = chrono::Utc::now().to_rfc3339();

        // Re-insert the main blob
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&blob_id)
        .bind(&auth.user_id)
        .bind(blob_type)
        .bind(row.size_bytes)
        .bind(&now)
        .bind(&row.file_path)
        .execute(&mut *tx)
        .await?;

        // Re-insert the thumbnail blob if present
        if let (Some(ref thumb_blob_id), Some(ref thumb_path)) =
            (&row.thumbnail_blob_id, &row.thumb_path)
        {
            // Thumbnail blobs are small — use 0 as size (client doesn't need it)
            sqlx::query(
                "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
                 VALUES (?, ?, 'thumbnail', 0, ?, ?)",
            )
            .bind(thumb_blob_id)
            .bind(&auth.user_id)
            .bind(&now)
            .bind(thumb_path)
            .execute(&mut *tx)
            .await?;
        }

        // Remove from trash
        sqlx::query("DELETE FROM trash_items WHERE id = ? AND user_id = ?")
            .bind(&trash_id)
            .bind(&auth.user_id)
            .execute(&mut *tx)
            .await?;

        tracing::info!("Encrypted blob {} restored from trash", blob_id);
    } else {
        // ── Plain photo restore (original logic) ────────────────────────────
        let item = sqlx::query_as::<_, TrashPhotoRow>(
            "SELECT photo_id AS id, filename, file_path, mime_type, media_type, size_bytes, \
             width, height, duration_secs, taken_at, latitude, longitude, thumb_path \
             FROM trash_items WHERE id = ? AND user_id = ?",
        )
        .bind(&trash_id)
        .bind(&auth.user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;

        let now = Utc::now().to_rfc3339();

        // Re-insert into photos
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
             thumb_path, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&item.id)
        .bind(&auth.user_id)
        .bind(&item.filename)
        .bind(&item.file_path)
        .bind(&item.mime_type)
        .bind(&item.media_type)
        .bind(item.size_bytes)
        .bind(item.width)
        .bind(item.height)
        .bind(item.duration_secs)
        .bind(&item.taken_at)
        .bind(item.latitude)
        .bind(item.longitude)
        .bind(&item.thumb_path)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        // Remove from trash
        sqlx::query("DELETE FROM trash_items WHERE id = ? AND user_id = ?")
            .bind(&trash_id)
            .bind(&auth.user_id)
            .execute(&mut *tx)
            .await?;

        tracing::info!("Photo {} restored from trash", item.id);
    }

    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/trash/:id
/// Permanently delete a single item from the trash (and its files on disk).
pub async fn permanent_delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(trash_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Begin transaction — ref-count check + DELETE must be atomic to prevent TOCTOU races
    let mut tx = state.pool.begin().await?;

    let item: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT file_path, thumb_path FROM trash_items WHERE id = ? AND user_id = ?",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (file_path, thumb_path) = item.ok_or(AppError::NotFound)?;

    // Only delete files from disk if no other photo row references the same
    // file_path (which happens when the user duplicates a photo via "Save Copy").
    let other_refs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE file_path = ?",
    )
    .bind(&file_path)
    .fetch_one(&mut *tx)
    .await?;

    let can_delete_file = other_refs == 0;

    let can_delete_thumb = if let Some(ref tp) = thumb_path {
        let other_thumb_refs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM photos WHERE thumb_path = ?",
        )
        .bind(tp)
        .fetch_one(&mut *tx)
        .await?;
        other_thumb_refs == 0
    } else {
        false
    };

    // Remove from database first (within the transaction)
    sqlx::query("DELETE FROM trash_items WHERE id = ? AND user_id = ?")
        .bind(&trash_id)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // Delete files from disk AFTER commit — a failure here is a minor storage
    // leak but preserves data integrity (the trash row is already gone).
    let storage_root = state.storage_root.read().await.clone();

    if can_delete_file {
        let full_path = storage_root.join(&file_path);
        if tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
            if let Err(e) = tokio::fs::remove_file(&full_path).await {
                tracing::warn!("Failed to delete photo file {}: {}", file_path, e);
            }
        }
    }

    if let Some(ref tp) = thumb_path {
        if can_delete_thumb {
            let thumb_full = storage_root.join(tp);
            if tokio::fs::try_exists(&thumb_full).await.unwrap_or(false) {
                if let Err(e) = tokio::fs::remove_file(&thumb_full).await {
                    tracing::warn!("Failed to delete thumbnail {}: {}", tp, e);
                }
            }
        }
    }

    tracing::info!("Permanently deleted trash item {}", trash_id);

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/trash
/// Empty the entire trash (permanently delete all items for this user).
pub async fn empty_trash(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Begin transaction — ref-count checks + batch DELETE must be atomic
    let mut tx = state.pool.begin().await?;

    // Fetch all trash items for file cleanup
    let items: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT file_path, thumb_path FROM trash_items WHERE user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&mut *tx)
    .await?;

    let deleted_count = items.len() as i64;

    // Build a list of files safe to delete (no other photo row references them).
    // We check within the transaction to avoid TOCTOU races.
    let mut files_to_delete: Vec<std::path::PathBuf> = Vec::new();
    let storage_root = state.storage_root.read().await.clone();

    for (file_path, thumb_path) in &items {
        let other_refs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM photos WHERE file_path = ?",
        )
        .bind(file_path)
        .fetch_one(&mut *tx)
        .await?;

        if other_refs == 0 {
            files_to_delete.push(storage_root.join(file_path));
        }

        if let Some(tp) = thumb_path {
            let other_thumb_refs: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM photos WHERE thumb_path = ?",
            )
            .bind(tp)
            .fetch_one(&mut *tx)
            .await?;

            if other_thumb_refs == 0 {
                files_to_delete.push(storage_root.join(tp));
            }
        }
    }

    // Remove all rows from database (within the transaction)
    sqlx::query("DELETE FROM trash_items WHERE user_id = ?")
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // Delete files from disk AFTER commit — failures here are minor storage
    // leaks but preserve data integrity.
    for path in &files_to_delete {
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            if let Err(e) = tokio::fs::remove_file(path).await {
                tracing::warn!("Failed to delete file {:?}: {}", path, e);
            }
        }
    }

    tracing::info!(
        "Emptied trash for user {}: {} items permanently deleted",
        auth.user_id,
        deleted_count
    );

    Ok(Json(serde_json::json!({
        "deleted": deleted_count,
        "message": format!("{} items permanently deleted", deleted_count),
    })))
}

/// GET /api/trash/:id/thumb
/// Serve the thumbnail for a trashed photo (so users can see what they're restoring).
pub async fn serve_trash_thumbnail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(trash_id): Path<String>,
) -> Result<Response, AppError> {
    let thumb_path: Option<String> = sqlx::query_scalar(
        "SELECT thumb_path FROM trash_items WHERE id = ? AND user_id = ?",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;
    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&thumb_path);

    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err(AppError::NotFound);
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
        .header(
            "Cache-Control",
            HeaderValue::from_static("private, max-age=86400"),
        )
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// Background task: purge expired trash items.
/// Called periodically (e.g. every hour) to permanently delete items
/// whose `expires_at` has passed.
pub async fn purge_expired_trash(pool: &sqlx::SqlitePool, storage_root: &std::path::Path) {
    let now = Utc::now().to_rfc3339();

    // Begin transaction — ref-count checks + batch DELETE must be atomic
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for trash purge: {}", e);
            return;
        }
    };

    // Fetch expired items for file cleanup
    let expired: Vec<(String, String, Option<String>)> = match sqlx::query_as(
        "SELECT id, file_path, thumb_path FROM trash_items WHERE expires_at <= ?",
    )
    .bind(&now)
    .fetch_all(&mut *tx)
    .await
    {
        Ok(items) => items,
        Err(e) => {
            tracing::error!("Failed to query expired trash items: {}", e);
            return;
        }
    };

    if expired.is_empty() {
        return;
    }

    // Build list of files safe to delete and IDs to remove, all within the transaction
    let mut files_to_delete: Vec<std::path::PathBuf> = Vec::new();
    let mut ids_to_delete: Vec<&str> = Vec::new();

    for (id, file_path, thumb_path) in &expired {
        // Only delete files if no other photo row still references them.
        // On DB error, skip this item entirely — do NOT default to 0
        // because that would cause irreversible file deletion.
        let other_refs: i64 = match sqlx::query_scalar(
            "SELECT COUNT(*) FROM photos WHERE file_path = ?",
        )
        .bind(file_path)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("DB error checking file refs for {}: {} — skipping", id, e);
                continue;
            }
        };

        if other_refs == 0 {
            files_to_delete.push(storage_root.join(file_path));
        }

        if let Some(tp) = thumb_path {
            let other_thumb_refs: i64 = match sqlx::query_scalar(
                "SELECT COUNT(*) FROM photos WHERE thumb_path = ?",
            )
            .bind(tp)
            .fetch_one(&mut *tx)
            .await
            {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!("DB error checking thumb refs for {}: {} — skipping", id, e);
                    continue;
                }
            };

            if other_thumb_refs == 0 {
                files_to_delete.push(storage_root.join(tp));
            }
        }

        ids_to_delete.push(id.as_str());
    }

    // Delete all expired rows within the transaction
    for id in &ids_to_delete {
        if let Err(e) = sqlx::query("DELETE FROM trash_items WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
        {
            tracing::error!("Failed to delete expired trash item {}: {}", id, e);
        }
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit trash purge transaction: {}", e);
        return;
    }

    // Delete files from disk AFTER commit
    for path in &files_to_delete {
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }

    let purged = ids_to_delete.len();
    if purged > 0 {
        tracing::info!("Purged {} expired trash items", purged);
    }
}

// ── Internal helper ─────────────────────────────────────────────────────────

/// Minimal row type used to move data between photos ↔ trash_items.
#[derive(Debug, sqlx::FromRow)]
struct TrashPhotoRow {
    id: String,
    filename: String,
    file_path: String,
    mime_type: String,
    media_type: String,
    size_bytes: i64,
    width: i64,
    height: i64,
    duration_secs: Option<f64>,
    taken_at: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    thumb_path: Option<String>,
}

/// Row type for restoring encrypted blob items from trash.
#[derive(Debug, sqlx::FromRow)]
struct TrashBlobRow {
    file_path: String,
    mime_type: String,
    media_type: String,
    size_bytes: i64,
    thumb_path: Option<String>,
    thumbnail_blob_id: Option<String>,
}
