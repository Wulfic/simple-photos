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
    pub encrypted_blob_id: Option<String>,
    pub encrypted_thumb_blob_id: Option<String>,
    pub is_favorite: bool,
    pub crop_metadata: Option<String>,
    pub camera_model: Option<String>,
    pub photo_hash: Option<String>,
}

/// Paginated list of plain-mode photos returned by `GET /api/photos`.
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

/// Response for the encryption settings endpoint.
#[derive(Debug, Serialize)]
pub struct EncryptionSettingsResponse {
    pub encryption_mode: String,
    pub migration_status: String,
    pub migration_total: i64,
    pub migration_completed: i64,
    pub migration_error: Option<String>,
}

/// Response for secure gallery listing.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SecureGalleryRecord {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub item_count: i64,
}

#[derive(Debug, Serialize)]
pub struct SecureGalleryListResponse {
    pub galleries: Vec<SecureGalleryRecord>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSecureGalleryRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UnlockSecureGalleryRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct SecureGalleryUnlockResponse {
    pub gallery_token: String,
    pub expires_in: u64,
}
