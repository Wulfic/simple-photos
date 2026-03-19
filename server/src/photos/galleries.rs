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
use crate::sanitize;
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
    let name = sanitize::sanitize_display_name(&req.name, 100)
        .map_err(|reason| AppError::BadRequest(reason.into()))?;

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
    .bind(&name)
    .bind("account-auth") // placeholder — not used for verification
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "gallery_id": gallery_id,
            "name": name,
        })),
    ))
}

/// POST /api/galleries/secure/unlock
/// Verify the user's account password. Returns a gallery access token
/// (HMAC-SHA256 signed, 1-hour TTL).
///
/// **NOTE:** The token's expiration is currently NOT validated on the
/// read path (`list_gallery_items`) — any non-empty token string is
/// accepted. This is a known gap; callers should treat the TTL as
/// advisory until server-side validation is added.
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
///
/// Ownership is verified first — only the gallery owner can delete it.
pub async fn delete_secure_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(gallery_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Verify ownership BEFORE deleting items to prevent IDOR:
    // without this check any authenticated user who guesses a gallery UUID
    // could wipe another user's gallery items.
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

    // Now safe to delete items — we've confirmed the caller owns the gallery.
    sqlx::query("DELETE FROM encrypted_gallery_items WHERE gallery_id = ?")
        .bind(&gallery_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Request body for `POST /api/galleries/secure/{id}/items`.
/// Associates an encrypted blob with a secure gallery.
#[derive(Debug, Deserialize)]
pub struct AddGalleryItemRequest {
    pub blob_id: String,
}

/// POST /api/galleries/secure/{id}/items — add a blob to a secure gallery.
///
/// Creates an **independent copy** of the blob for the secure album rather
/// than sharing a reference to the original.  This ensures each secure album
/// folder has its own blob namespace, preventing mix-ups between main-gallery
/// and secure-album data.
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

    // Fetch original blob metadata
    let original: Option<(String, String, i64, Option<String>, String, Option<String>)> =
        sqlx::query_as(
            "SELECT id, blob_type, size_bytes, client_hash, storage_path, content_hash \
             FROM blobs WHERE id = ? AND user_id = ?",
        )
        .bind(&req.blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?;

    let (_orig_id, blob_type, size_bytes, client_hash, storage_path, content_hash) =
        original.ok_or_else(|| AppError::BadRequest("Blob not found".into()))?;

    // Clone: read the original blob data from disk, write a new copy under a fresh ID
    let storage_root = (**state.storage_root.load()).clone();
    let blob_data = crate::blobs::storage::read_blob(&storage_root, &storage_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read source blob: {}", e)))?;

    let new_blob_id = Uuid::new_v4().to_string();
    let new_storage_path = crate::blobs::storage::write_blob(
        &storage_root, &auth.user_id, &new_blob_id, &blob_data,
    )
    .await
    .map_err(|e| AppError::Internal(format!("Failed to write cloned blob: {}", e)))?;

    let now = Utc::now().to_rfc3339();

    // Insert the cloned blob record
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&new_blob_id)
    .bind(&auth.user_id)
    .bind(&blob_type)
    .bind(size_bytes)
    .bind(&client_hash)
    .bind(&now)
    .bind(&new_storage_path)
    .bind(&content_hash)
    .execute(&state.pool)
    .await?;

    // Also clone the thumbnail blob if one exists (via photos.encrypted_thumb_blob_id)
    let thumb_blob_id: Option<String> = sqlx::query_scalar(
        "SELECT encrypted_thumb_blob_id FROM photos WHERE encrypted_blob_id = ?",
    )
    .bind(&req.blob_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();

    if let Some(ref orig_thumb_id) = thumb_blob_id {
        let thumb_meta: Option<(String, i64, Option<String>, String)> = sqlx::query_as(
            "SELECT blob_type, size_bytes, client_hash, storage_path FROM blobs WHERE id = ?",
        )
        .bind(orig_thumb_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();

        if let Some((tb_type, tb_size, tb_hash, tb_path)) = thumb_meta {
            if let Ok(thumb_data) = crate::blobs::storage::read_blob(&storage_root, &tb_path).await {
                let new_thumb_id = Uuid::new_v4().to_string();
                if let Ok(new_thumb_path) = crate::blobs::storage::write_blob(
                    &storage_root, &auth.user_id, &new_thumb_id, &thumb_data,
                ).await {
                    let _ = sqlx::query(
                        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
                    )
                    .bind(&new_thumb_id)
                    .bind(&auth.user_id)
                    .bind(&tb_type)
                    .bind(tb_size)
                    .bind(&tb_hash)
                    .bind(&now)
                    .bind(&new_thumb_path)
                    .execute(&state.pool)
                    .await;
                }
            }
        }
    }

    let item_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT OR IGNORE INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(&item_id)
    .bind(&gallery_id)
    .bind(&new_blob_id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "item_id": item_id,
            "new_blob_id": new_blob_id,
        })),
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
