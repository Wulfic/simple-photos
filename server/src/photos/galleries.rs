//! Encrypted gallery management endpoints.

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

/// GET /api/galleries/encrypted
/// List encrypted galleries for the authenticated user.
pub async fn list_encrypted_galleries(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<EncryptedGalleryListResponse>, AppError> {
    let galleries = sqlx::query_as::<_, EncryptedGalleryRecord>(
        "SELECT g.id, g.name, g.created_at, \
         (SELECT COUNT(*) FROM encrypted_gallery_items WHERE gallery_id = g.id) as item_count \
         FROM encrypted_galleries g WHERE g.user_id = ? ORDER BY g.created_at DESC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(EncryptedGalleryListResponse { galleries }))
}

/// POST /api/galleries/encrypted
/// Create a new encrypted gallery with its own password.
pub async fn create_encrypted_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateEncryptedGalleryRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    if req.name.is_empty() || req.name.len() > 100 {
        return Err(AppError::BadRequest(
            "Gallery name must be 1-100 characters".into(),
        ));
    }
    if req.password.len() < 4 {
        return Err(AppError::BadRequest(
            "Gallery password must be at least 4 characters".into(),
        ));
    }

    let gallery_id = Uuid::new_v4().to_string();
    let password_hash = bcrypt::hash(&req.password, 10)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO encrypted_galleries (id, user_id, name, password_hash, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .bind(&req.name)
    .bind(&password_hash)
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

/// POST /api/galleries/encrypted/:id/unlock
/// Verify gallery password. Returns a short-lived token for accessing gallery items.
pub async fn unlock_encrypted_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(gallery_id): Path<String>,
    Json(req): Json<UnlockEncryptedGalleryRequest>,
) -> Result<Json<EncryptedGalleryUnlockResponse>, AppError> {
    let password_hash: String = sqlx::query_scalar(
        "SELECT password_hash FROM encrypted_galleries WHERE id = ? AND user_id = ?",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let valid = bcrypt::verify(&req.password, &password_hash)
        .map_err(|e| AppError::Internal(format!("Bcrypt verify failed: {}", e)))?;

    if !valid {
        return Err(AppError::Unauthorized("Invalid gallery password".into()));
    }

    let now = Utc::now().timestamp();
    let expires_in = 3600u64;
    let payload = format!("{}:{}:{}", gallery_id, auth.user_id, now);
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    hasher.update(state.config.auth.jwt_secret.as_bytes());
    let token = format!("gal_{}_{}", now, hex::encode(hasher.finalize()));

    Ok(Json(EncryptedGalleryUnlockResponse {
        gallery_token: token,
        expires_in,
    }))
}

/// DELETE /api/galleries/encrypted/:id
/// Delete an encrypted gallery and its items.
pub async fn delete_encrypted_gallery(
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

/// POST /api/galleries/encrypted/:id/items
/// Add a blob to an encrypted gallery.
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

/// GET /api/galleries/encrypted/:id/items
/// List items in an encrypted gallery (requires unlock token in header).
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
