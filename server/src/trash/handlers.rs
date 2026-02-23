use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
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
             deleted_at, expires_at \
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
             deleted_at, expires_at \
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
/// inserted into `trash_items` with a 30-day expiration.
pub async fn soft_delete_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Fetch the photo record first
    let photo = sqlx::query_as::<_, TrashPhotoRow>(
        "SELECT id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    // Read retention days from server_settings (default 30)
    let retention_days: i64 = sqlx::query_scalar(
        "SELECT CAST(value AS INTEGER) FROM server_settings WHERE key = 'trash_retention_days'",
    )
    .fetch_optional(&state.pool)
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
    .execute(&state.pool)
    .await?;

    // Remove from photos table
    sqlx::query("DELETE FROM photos WHERE id = ? AND user_id = ?")
        .bind(&photo_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    tracing::info!(
        "Photo {} moved to trash (expires {})",
        photo_id,
        expires_at.to_rfc3339()
    );

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/trash/:id/restore
/// Restore a photo from the trash back to the photos table.
pub async fn restore_from_trash(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(trash_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let item = sqlx::query_as::<_, TrashPhotoRow>(
        "SELECT photo_id AS id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path \
         FROM trash_items WHERE id = ? AND user_id = ?",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
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
    .execute(&state.pool)
    .await?;

    // Remove from trash
    sqlx::query("DELETE FROM trash_items WHERE id = ? AND user_id = ?")
        .bind(&trash_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    tracing::info!("Photo {} restored from trash", item.id);

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/trash/:id
/// Permanently delete a single item from the trash (and its files on disk).
pub async fn permanent_delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(trash_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let item: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT file_path, thumb_path FROM trash_items WHERE id = ? AND user_id = ?",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let (file_path, thumb_path) = item.ok_or(AppError::NotFound)?;

    // Delete files from disk
    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&file_path);
    if tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        if let Err(e) = tokio::fs::remove_file(&full_path).await {
            tracing::warn!("Failed to delete photo file {}: {}", file_path, e);
        }
    }

    if let Some(ref tp) = thumb_path {
        let thumb_full = storage_root.join(tp);
        if tokio::fs::try_exists(&thumb_full).await.unwrap_or(false) {
            if let Err(e) = tokio::fs::remove_file(&thumb_full).await {
                tracing::warn!("Failed to delete thumbnail {}: {}", tp, e);
            }
        }
    }

    // Remove from database
    sqlx::query("DELETE FROM trash_items WHERE id = ? AND user_id = ?")
        .bind(&trash_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    tracing::info!("Permanently deleted trash item {}", trash_id);

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/trash
/// Empty the entire trash (permanently delete all items for this user).
pub async fn empty_trash(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Fetch all trash items for file cleanup
    let items: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT file_path, thumb_path FROM trash_items WHERE user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let storage_root = state.storage_root.read().await.clone();
    let mut deleted_count = 0i64;

    for (file_path, thumb_path) in &items {
        let full_path = storage_root.join(file_path);
        if tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
            let _ = tokio::fs::remove_file(&full_path).await;
        }
        if let Some(tp) = thumb_path {
            let thumb_full = storage_root.join(tp);
            if tokio::fs::try_exists(&thumb_full).await.unwrap_or(false) {
                let _ = tokio::fs::remove_file(&thumb_full).await;
            }
        }
        deleted_count += 1;
    }

    // Remove all from database
    sqlx::query("DELETE FROM trash_items WHERE user_id = ?")
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

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

    // Fetch expired items for file cleanup
    let expired: Vec<(String, String, Option<String>)> = match sqlx::query_as(
        "SELECT id, file_path, thumb_path FROM trash_items WHERE expires_at <= ?",
    )
    .bind(&now)
    .fetch_all(pool)
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

    let mut purged = 0i64;

    for (id, file_path, thumb_path) in &expired {
        // Delete files from disk
        let full_path = storage_root.join(file_path);
        if tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
            let _ = tokio::fs::remove_file(&full_path).await;
        }
        if let Some(tp) = thumb_path {
            let thumb_full = storage_root.join(tp);
            if tokio::fs::try_exists(&thumb_full).await.unwrap_or(false) {
                let _ = tokio::fs::remove_file(&thumb_full).await;
            }
        }

        // Remove from database
        if let Err(e) = sqlx::query("DELETE FROM trash_items WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await
        {
            tracing::error!("Failed to delete expired trash item {}: {}", id, e);
            continue;
        }

        purged += 1;
    }

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
