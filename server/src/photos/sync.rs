//! Encrypted-mode sync endpoint.
//!
//! Returns photo metadata from the `photos` table for photos that have been
//! encrypted (have `encrypted_blob_id`). This lets mobile clients populate
//! their gallery without downloading and decrypting every full-size photo blob.
//!
//! Clients then download only the small thumbnail blobs (~30 KB each) for
//! gallery grid display and load full photos on-demand when viewed.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// Query parameters for the encrypted sync endpoint.
#[derive(Debug, Deserialize)]
pub struct SyncQuery {
    pub after: Option<String>,
    pub limit: Option<i64>,
}

/// Photo metadata record for encrypted-mode sync (no file content).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EncryptedSyncRecord {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
    pub created_at: String,
    /// NULL for photos registered by the autoscan pipeline that have not yet
    /// been uploaded as an encrypted blob by a client.
    pub encrypted_blob_id: Option<String>,
    pub encrypted_thumb_blob_id: Option<String>,
    pub is_favorite: bool,
    pub crop_metadata: Option<String>,
    pub photo_hash: Option<String>,
}

/// Paginated response from `GET /api/photos/encrypted-sync`.
#[derive(Debug, Serialize)]
pub struct EncryptedSyncResponse {
    pub photos: Vec<EncryptedSyncRecord>,
    pub next_cursor: Option<String>,
}

/// GET /api/photos/encrypted-sync
/// Returns metadata for encrypted photos — lightweight sync for mobile clients.
pub async fn encrypted_sync(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<SyncQuery>,
) -> Result<Json<EncryptedSyncResponse>, AppError> {
    let limit = params.limit.unwrap_or(500).min(1000);

    let photos = if let Some(ref after) = params.after {
        sqlx::query_as::<_, EncryptedSyncRecord>(
            "SELECT id, filename, mime_type, media_type, size_bytes, width, height, \
             duration_secs, taken_at, created_at, encrypted_blob_id, encrypted_thumb_blob_id, \
             is_favorite, crop_metadata, photo_hash \
             FROM photos \
             WHERE user_id = ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
             AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
             AND COALESCE(taken_at, created_at) < ? \
             ORDER BY COALESCE(taken_at, created_at) DESC, filename ASC \
             LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(after)
        .bind(limit + 1)
        .fetch_all(&state.read_pool)
        .await?
    } else {
        sqlx::query_as::<_, EncryptedSyncRecord>(
            "SELECT id, filename, mime_type, media_type, size_bytes, width, height, \
             duration_secs, taken_at, created_at, encrypted_blob_id, encrypted_thumb_blob_id, \
             is_favorite, crop_metadata, photo_hash \
             FROM photos \
             WHERE user_id = ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
             AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
             ORDER BY COALESCE(taken_at, created_at) DESC, filename ASC \
             LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(limit + 1)
        .fetch_all(&state.read_pool)
        .await?
    };

    let next_cursor = if photos.len() as i64 > limit {
        photos
            .last()
            .map(|p| p.taken_at.clone().unwrap_or_else(|| p.created_at.clone()))
    } else {
        None
    };

    let photos: Vec<EncryptedSyncRecord> = photos.into_iter().take(limit as usize).collect();

    Ok(Json(EncryptedSyncResponse {
        photos,
        next_cursor,
    }))
}
