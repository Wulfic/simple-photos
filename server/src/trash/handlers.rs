//! Trash read endpoints: list and thumbnail serving.
//!
//! Mutation operations (soft-delete, restore, permanent-delete, empty)
//! live in [`super::operations`].

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;

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
             deleted_at, expires_at, encrypted_blob_id, thumbnail_blob_id \
             FROM trash_items WHERE user_id = ? AND deleted_at < ? \
             ORDER BY deleted_at DESC LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(after)
        .bind(limit + 1)
        .fetch_all(&state.read_pool)
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
        .fetch_all(&state.read_pool)
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

/// GET /api/trash/:id/thumb
/// Serve the thumbnail for a trashed photo (so users can see what they're restoring).
pub async fn serve_trash_thumbnail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(trash_id): Path<String>,
) -> Result<Response, AppError> {
    let thumb_path: Option<String> =
        sqlx::query_scalar("SELECT thumb_path FROM trash_items WHERE id = ? AND user_id = ?")
            .bind(&trash_id)
            .bind(&auth.user_id)
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;
    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&thumb_path);

    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err(AppError::NotFound);
    }

    let meta = tokio::fs::metadata(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read thumbnail: {}", e)))?;
    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to open thumbnail: {}", e)))?;

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
