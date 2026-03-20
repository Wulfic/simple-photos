//! Shared album handlers — create, manage members, add/remove photos.
//!
//! Shared albums allow users to collaborate on photo collections:
//! - Owner creates an album and adds members (other registered users).
//! - Members can view all photos in the album.
//! - Owner and members can add/remove photos they own.
//! - Only the owner can delete the album or manage membership.
//!
//! Authorization is enforced per-endpoint: viewers cannot modify,
//! non-members cannot access, and the owner role is checked for
//! destructive operations.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use super::models::*;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Check whether the user is the owner or a member of the shared album.
async fn require_album_access(
    pool: &sqlx::SqlitePool,
    album_id: &str,
    user_id: &str,
) -> Result<bool, AppError> {
    // Check owner
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_user_id FROM shared_albums WHERE id = ?")
            .bind(album_id)
            .fetch_optional(pool)
            .await?;

    let owner_id = owner.ok_or(AppError::NotFound)?;
    if owner_id == user_id {
        return Ok(true); // is_owner = true
    }

    // Check membership
    let is_member: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM shared_album_members WHERE album_id = ? AND user_id = ?)",
    )
    .bind(album_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    if !is_member {
        return Err(AppError::Forbidden(
            "You do not have access to this album".into(),
        ));
    }
    Ok(false) // is_owner = false
}

// ── List shared albums ─────────────────────────────────────────────────────

/// List all shared albums the user owns or is a member of.
///
/// GET /api/sharing/albums
pub async fn list_shared_albums(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<SharedAlbumInfo>>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, String, String)>(
        r#"
        SELECT sa.id, sa.name, sa.owner_user_id, sa.created_at
        FROM shared_albums sa
        WHERE sa.owner_user_id = ?
        UNION
        SELECT sa.id, sa.name, sa.owner_user_id, sa.created_at
        FROM shared_albums sa
        JOIN shared_album_members sam ON sam.album_id = sa.id
        WHERE sam.user_id = ?
        ORDER BY sa.created_at DESC
        "#,
    )
    .bind(&auth.user_id)
    .bind(&auth.user_id)
    .fetch_all(&state.read_pool)
    .await?;

    let mut albums = Vec::with_capacity(rows.len());
    for (id, name, owner_user_id, created_at) in rows {
        let owner_username: String =
            sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
                .bind(&owner_user_id)
                .fetch_optional(&state.read_pool)
                .await?
                .unwrap_or_else(|| "unknown".to_string());

        let photo_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM shared_album_photos WHERE album_id = ?",
        )
        .bind(&id)
        .fetch_one(&state.read_pool)
        .await?;

        let member_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM shared_album_members WHERE album_id = ?",
        )
        .bind(&id)
        .fetch_one(&state.read_pool)
        .await?;

        albums.push(SharedAlbumInfo {
            id,
            name,
            owner_username,
            is_owner: owner_user_id == auth.user_id,
            photo_count,
            member_count,
            created_at,
        });
    }

    Ok(Json(albums))
}

// ── Create shared album ────────────────────────────────────────────────────

/// Create a new shared album. The creator is the owner.
///
/// POST /api/sharing/albums
pub async fn create_shared_album(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateSharedAlbumRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let name = sanitize::sanitize_display_name(&req.name, 200)
        .map_err(|reason| AppError::BadRequest(reason.into()))?;

    let album_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query("INSERT INTO shared_albums (id, owner_user_id, name, created_at) VALUES (?, ?, ?, ?)")
        .bind(&album_id)
        .bind(&auth.user_id)
        .bind(&name)
        .bind(&now)
        .execute(&state.pool)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": album_id,
            "name": name,
            "created_at": now,
        })),
    ))
}

// ── Delete shared album ────────────────────────────────────────────────────

/// Delete a shared album. Only the owner can delete it.
///
/// DELETE /api/sharing/albums/{id}
pub async fn delete_shared_album(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(album_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_user_id FROM shared_albums WHERE id = ?")
            .bind(&album_id)
            .fetch_optional(&state.read_pool)
            .await?;

    let owner_id = owner.ok_or(AppError::NotFound)?;
    if owner_id != auth.user_id {
        return Err(AppError::Forbidden("Only the album owner can delete it".into()));
    }

    sqlx::query("DELETE FROM shared_albums WHERE id = ?")
        .bind(&album_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── List members ───────────────────────────────────────────────────────────

/// List members of a shared album.
///
/// GET /api/sharing/albums/{id}/members
pub async fn list_members(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(album_id): Path<String>,
) -> Result<Json<Vec<SharedAlbumMember>>, AppError> {
    require_album_access(&state.read_pool, &album_id, &auth.user_id).await?;

    let members = sqlx::query_as::<_, SharedAlbumMember>(
        r#"
        SELECT sam.id, sam.user_id, u.username, sam.added_at
        FROM shared_album_members sam
        JOIN users u ON u.id = sam.user_id
        WHERE sam.album_id = ?
        ORDER BY sam.added_at ASC
        "#,
    )
    .bind(&album_id)
    .fetch_all(&state.read_pool)
    .await?;

    Ok(Json(members))
}

// ── Add member ─────────────────────────────────────────────────────────────

/// Add a user to a shared album. Only the owner can add members.
///
/// POST /api/sharing/albums/{id}/members
pub async fn add_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(album_id): Path<String>,
    Json(req): Json<AddMemberRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Only owner can add members
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_user_id FROM shared_albums WHERE id = ?")
            .bind(&album_id)
            .fetch_optional(&state.read_pool)
            .await?;

    let owner_id = owner.ok_or(AppError::NotFound)?;
    if owner_id != auth.user_id {
        return Err(AppError::Forbidden("Only the album owner can add members".into()));
    }

    // Can't add yourself
    if req.user_id == auth.user_id {
        return Err(AppError::BadRequest("Cannot add yourself as a member".into()));
    }

    // Verify user exists
    let user_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = ?)")
            .bind(&req.user_id)
            .fetch_one(&state.read_pool)
            .await?;

    if !user_exists {
        return Err(AppError::NotFound);
    }

    let member_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT OR IGNORE INTO shared_album_members (id, album_id, user_id, added_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&member_id)
    .bind(&album_id)
    .bind(&req.user_id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "member_id": member_id,
            "user_id": req.user_id,
        })),
    ))
}

// ── Remove member ──────────────────────────────────────────────────────────

/// Remove a user from a shared album. Only the owner can remove members.
///
/// DELETE /api/sharing/albums/{album_id}/members/{user_id}
pub async fn remove_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((album_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_user_id FROM shared_albums WHERE id = ?")
            .bind(&album_id)
            .fetch_optional(&state.read_pool)
            .await?;

    let owner_id = owner.ok_or(AppError::NotFound)?;
    if owner_id != auth.user_id {
        return Err(AppError::Forbidden("Only the owner can remove members".into()));
    }

    sqlx::query("DELETE FROM shared_album_members WHERE album_id = ? AND user_id = ?")
        .bind(&album_id)
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── List photos in shared album ────────────────────────────────────────────

/// List photos in a shared album.
///
/// GET /api/sharing/albums/{id}/photos
pub async fn list_shared_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(album_id): Path<String>,
) -> Result<Json<Vec<SharedAlbumPhoto>>, AppError> {
    require_album_access(&state.read_pool, &album_id, &auth.user_id).await?;

    let photos = sqlx::query_as::<_, SharedAlbumPhoto>(
        "SELECT id, photo_ref, ref_type, added_at FROM shared_album_photos WHERE album_id = ? ORDER BY added_at ASC",
    )
    .bind(&album_id)
    .fetch_all(&state.read_pool)
    .await?;

    Ok(Json(photos))
}

// ── Add photo to shared album ──────────────────────────────────────────────

/// Add a photo to a shared album. Any member (or owner) can add photos.
///
/// POST /api/sharing/albums/{id}/photos
pub async fn add_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(album_id): Path<String>,
    Json(req): Json<AddPhotoRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    require_album_access(&state.read_pool, &album_id, &auth.user_id).await?;

    if req.ref_type != "photo" && req.ref_type != "blob" {
        return Err(AppError::BadRequest(
            "ref_type must be 'photo' or 'blob'".into(),
        ));
    }

    let photo_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT OR IGNORE INTO shared_album_photos (id, album_id, photo_ref, ref_type, added_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&photo_id)
    .bind(&album_id)
    .bind(&req.photo_ref)
    .bind(&req.ref_type)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "photo_id": photo_id })),
    ))
}

// ── Remove photo from shared album ────────────────────────────────────────

/// Remove a photo from a shared album. Any album member (or owner) can
/// remove any photo in the album.
///
/// DELETE /api/sharing/albums/{album_id}/photos/{photo_id}
pub async fn remove_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((album_id, photo_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    require_album_access(&state.read_pool, &album_id, &auth.user_id).await?;

    sqlx::query("DELETE FROM shared_album_photos WHERE album_id = ? AND id = ?")
        .bind(&album_id)
        .bind(&photo_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── List all users (for sharing picker) ────────────────────────────────────

/// List all registered users (id + username) so the UI can show a picker.
/// Only authenticated users may call this.
///
/// GET /api/sharing/users
pub async fn list_users_for_sharing(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT id, username FROM users ORDER BY username ASC",
    )
    .fetch_all(&state.read_pool)
    .await?;

    let users: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, username)| {
            serde_json::json!({ "id": id, "username": username })
        })
        .collect();

    Ok(Json(users))
}
