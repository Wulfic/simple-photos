use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Serialize)]
pub struct PhotoListResponse {
    pub photos: Vec<PhotoRecord>,
    pub next_cursor: Option<String>,
}

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
}

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

/// Response for encrypted gallery listing.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EncryptedGalleryRecord {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub item_count: i64,
}

#[derive(Debug, Serialize)]
pub struct EncryptedGalleryListResponse {
    pub galleries: Vec<EncryptedGalleryRecord>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEncryptedGalleryRequest {
    pub name: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct UnlockEncryptedGalleryRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct EncryptedGalleryUnlockResponse {
    pub gallery_token: String,
    pub expires_in: u64,
}
