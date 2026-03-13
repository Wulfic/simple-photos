//! DTOs for the shared albums feature.

use serde::{Deserialize, Serialize};

/// A shared album with ownership info.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SharedAlbum {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub created_at: String,
}

/// Shared album with extra metadata for listing.
#[derive(Debug, Serialize)]
pub struct SharedAlbumInfo {
    pub id: String,
    pub name: String,
    pub owner_username: String,
    pub is_owner: bool,
    pub photo_count: i64,
    pub member_count: i64,
    pub created_at: String,
}

/// A member of a shared album.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SharedAlbumMember {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub added_at: String,
}

/// A photo reference within a shared album.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SharedAlbumPhoto {
    pub id: String,
    pub photo_ref: String,
    pub ref_type: String,
    pub added_at: String,
}

// ── Request types ──────────────────────────────────────────────────────────

/// Request body for `POST /api/sharing/albums`.
#[derive(Debug, Deserialize)]
pub struct CreateSharedAlbumRequest {
    pub name: String,
}

/// Request body for `POST /api/sharing/albums/:id/members`.
#[derive(Debug, Deserialize)]
pub struct AddMemberRequest {
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct AddPhotoRequest {
    pub photo_ref: String,
    /// "plain" (photos table) or "blob" (blobs table)
    #[serde(default = "default_ref_type")]
    pub ref_type: String,
}

fn default_ref_type() -> String {
    "plain".to_string()
}
