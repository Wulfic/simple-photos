//! DTOs for the trash (soft-delete) system.

use serde::{Deserialize, Serialize};

/// A photo in the trash bin, pending permanent deletion.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TrashItem {
    pub id: String,
    pub photo_id: String,
    pub filename: String,
    pub file_path: String,
    pub mime_type: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub thumb_path: Option<String>,
    pub deleted_at: String,
    pub expires_at: String,
    /// If set, this trash item came from an encrypted blob (not a plain photo).
    pub encrypted_blob_id: Option<String>,
    /// The companion thumbnail blob ID (encrypted mode only).
    pub thumbnail_blob_id: Option<String>,
}

/// Paginated trash listing response.
#[derive(Debug, Serialize)]
pub struct TrashListResponse {
    pub items: Vec<TrashItem>,
    pub next_cursor: Option<String>,
}

/// Query parameters for `GET /api/trash`.
#[derive(Debug, Deserialize)]
pub struct TrashListQuery {
    pub after: Option<String>,
    pub limit: Option<i64>,
}

/// Request body sent by the client when soft-deleting an encrypted blob.
/// The server doesn't know the metadata for encrypted blobs, so the client
/// provides it.
#[derive(Debug, Deserialize)]
pub struct SoftDeleteBlobRequest {
    pub thumbnail_blob_id: Option<String>,
    pub filename: String,
    pub mime_type: String,
    pub media_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
}
