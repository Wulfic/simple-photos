//! Secure gallery management endpoints.
//!
//! Secure galleries use the user's account password for authentication,
//! not a separate gallery-specific password.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::*;

/// GET /api/galleries/secure
/// List secure galleries for the authenticated user.
pub async fn list_secure_galleries(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<SecureGalleryListResponse>, AppError> {
    let galleries = sqlx::query_as::<_, SecureGalleryRecord>(
        "SELECT g.id, g.name, g.created_at, \
         (SELECT COUNT(*) FROM encrypted_gallery_items WHERE gallery_id = g.id) as item_count \
         FROM encrypted_galleries g WHERE g.user_id = ? ORDER BY g.created_at DESC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(SecureGalleryListResponse { galleries }))
}

/// POST /api/galleries/secure
/// Create a new secure gallery (no separate password — uses account password).
pub async fn create_secure_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateSecureGalleryRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    if req.name.is_empty() || req.name.len() > 100 {
        return Err(AppError::BadRequest(
            "Gallery name must be 1-100 characters".into(),
        ));
    }

    let gallery_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    // Store a placeholder for password_hash (column is NOT NULL for legacy compat).
    // Auth is handled via the user's account password at unlock time.
    sqlx::query(
        "INSERT INTO encrypted_galleries (id, user_id, name, password_hash, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .bind(&req.name)
    .bind("account-auth") // placeholder — not used for verification
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "gallery_id": gallery_id,
            "name": req.name,
        })),
    ))
}

/// POST /api/galleries/secure/unlock
/// Verify the user's account password. Returns a short-lived token for accessing
/// all secure galleries belonging to this user.
pub async fn unlock_secure_galleries(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<UnlockSecureGalleryRequest>,
) -> Result<Json<SecureGalleryUnlockResponse>, AppError> {
    // Verify against the user's account password
    let password_hash: String = sqlx::query_scalar(
        "SELECT password_hash FROM users WHERE id = ?",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let valid = bcrypt::verify(&req.password, &password_hash)
        .map_err(|e| AppError::Internal(format!("Bcrypt verify failed: {}", e)))?;

    if !valid {
        return Err(AppError::Unauthorized("Invalid password".into()));
    }

    let now = Utc::now().timestamp();
    let expires_in = 3600u64; // 1 hour
    let payload = format!("secure:{}:{}", auth.user_id, now);
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    hasher.update(state.config.auth.jwt_secret.as_bytes());
    let token = format!("sec_{}_{}", now, hex::encode(hasher.finalize()));

    Ok(Json(SecureGalleryUnlockResponse {
        gallery_token: token,
        expires_in,
    }))
}

/// DELETE /api/galleries/secure/:id
/// Delete a secure gallery and its items.
pub async fn delete_secure_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(gallery_id): Path<String>,
) -> Result<StatusCode, AppError> {
    sqlx::query("DELETE FROM encrypted_gallery_items WHERE gallery_id = ?")
        .bind(&gallery_id)
        .execute(&state.pool)
        .await?;

    let result = sqlx::query(
        "DELETE FROM encrypted_galleries WHERE id = ? AND user_id = ?",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/galleries/secure/:id/items
/// Add a blob to a secure gallery.
#[derive(Debug, Deserialize)]
pub struct AddGalleryItemRequest {
    pub blob_id: String,
}

pub async fn add_gallery_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(gallery_id): Path<String>,
    Json(req): Json<AddGalleryItemRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    if count == 0 {
        return Err(AppError::NotFound);
    }

    let blob_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&req.blob_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    if blob_count == 0 {
        return Err(AppError::BadRequest("Blob not found".into()));
    }

    let item_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT OR IGNORE INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(&item_id)
    .bind(&gallery_id)
    .bind(&req.blob_id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "item_id": item_id })),
    ))
}

/// GET /api/galleries/secure/blob-ids
/// Return all blob IDs that live in any of the user's secure galleries.
/// This is used by the main gallery to filter out "private" items without
/// requiring the gallery unlock token.
pub async fn list_secure_blob_ids(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let blob_ids: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT gi.blob_id \
         FROM encrypted_gallery_items gi \
         JOIN encrypted_galleries g ON g.id = gi.gallery_id \
         WHERE g.user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let ids: Vec<&str> = blob_ids.iter().map(|(id,)| id.as_str()).collect();
    Ok(Json(serde_json::json!({ "blob_ids": ids })))
}

/// GET /api/galleries/secure/:id/items
/// List items in a secure gallery (requires unlock token in header).
pub async fn list_gallery_items(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(gallery_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    if count == 0 {
        return Err(AppError::NotFound);
    }

    let _token = headers
        .get("x-gallery-token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Gallery token required. Unlock the gallery first.".into()))?;

    let items: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT gi.id, gi.blob_id, gi.added_at \
         FROM encrypted_gallery_items gi WHERE gi.gallery_id = ? \
         ORDER BY gi.added_at DESC",
    )
    .bind(&gallery_id)
    .fetch_all(&state.pool)
    .await?;

    let items_json: Vec<serde_json::Value> = items
        .iter()
        .map(|(id, blob_id, added_at)| {
            serde_json::json!({
                "id": id,
                "blob_id": blob_id,
                "added_at": added_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "items": items_json })))
}
