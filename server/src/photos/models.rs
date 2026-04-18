//! Request/response DTOs and database row types for the photos subsystem.

use serde::{Deserialize, Serialize};

/// Full photo row from the `photos` table.
/// Used internally (e.g. duplication). Not directly serialized to API clients
/// — see [`PhotoRecord`] for the public-facing subset.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Photo {
    pub id: String,
    pub user_id: String,
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
    pub created_at: String,
    pub encrypted_blob_id: String,
    pub encrypted_thumb_blob_id: Option<String>,
    pub is_favorite: bool,
    pub crop_metadata: Option<String>,
    pub camera_model: Option<String>,
    pub photo_hash: Option<String>,
    pub photo_subtype: Option<String>,
    pub burst_id: Option<String>,
    pub motion_video_blob_id: Option<String>,
}

/// Paginated list of photos returned by `GET /api/photos`.
#[derive(Debug, Serialize)]
pub struct PhotoListResponse {
    pub photos: Vec<PhotoRecord>,
    pub next_cursor: Option<String>,
}

/// Public-facing photo record (excludes `user_id` and encryption internals).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PhotoRecord {
    pub id: String,
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
    pub created_at: String,
    pub is_favorite: bool,
    pub crop_metadata: Option<String>,
    pub camera_model: Option<String>,
    pub photo_hash: Option<String>,
    pub photo_subtype: Option<String>,
    pub burst_id: Option<String>,
    pub motion_video_blob_id: Option<String>,
    /// Number of photos in this burst group. Populated when `collapse_bursts=true`;
    /// NULL otherwise.
    pub burst_count: Option<i64>,
}

/// Request body for `POST /api/photos/register`.
/// Registers an existing file on disk as a photo in the database.
#[derive(Debug, Deserialize)]
pub struct RegisterPhotoRequest {
    pub filename: String,
    pub file_path: String,
    pub mime_type: String,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Request body for `POST /api/photos/register-encrypted`.
/// Creates a photos record linked to already-uploaded encrypted blobs.
#[derive(Debug, Deserialize)]
pub struct RegisterEncryptedPhotoRequest {
    pub filename: String,
    pub mime_type: String,
    pub media_type: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub encrypted_blob_id: String,
    pub encrypted_thumb_blob_id: Option<String>,
    pub photo_hash: Option<String>,
    pub photo_subtype: Option<String>,
    pub burst_id: Option<String>,
    pub motion_video_blob_id: Option<String>,
}

/// Response for secure gallery listing.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SecureGalleryRecord {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub item_count: i64,
}

/// Response body for `GET /api/galleries/secure` — wraps the gallery list.
#[derive(Debug, Serialize)]
pub struct SecureGalleryListResponse {
    pub galleries: Vec<SecureGalleryRecord>,
}

/// Request body for `POST /api/galleries/secure` — create a new secure gallery.
#[derive(Debug, Deserialize)]
pub struct CreateSecureGalleryRequest {
    pub name: String,
}

/// Request body for `POST /api/galleries/secure/unlock`.
/// Verifies the gallery password and returns a short-lived access token.
#[derive(Debug, Deserialize)]
pub struct UnlockSecureGalleryRequest {
    pub password: String,
}

/// Response body for successful gallery unlock — contains a time-limited
/// token that must be sent as `X-Gallery-Token` to access gallery items.
#[derive(Debug, Serialize)]
pub struct SecureGalleryUnlockResponse {
    pub gallery_token: String,
    pub expires_in: u64,
}
