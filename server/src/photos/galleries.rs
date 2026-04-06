//! Secure gallery management endpoints.
//!
//! Secure galleries use the user's account password for authentication,
//! not a separate gallery-specific password.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use sqlx::FromRow;
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
    let password_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
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
    let result = sqlx::query("DELETE FROM encrypted_galleries WHERE id = ? AND user_id = ?")
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
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?")
            .bind(&gallery_id)
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    if count == 0 {
        return Err(AppError::NotFound);
    }

    // Fetch original blob metadata — first try the `blobs` table (encrypted
    // uploads), then fall back to the `photos` table (autoscanned/server-side
    // files).  The client may pass either a blob ID or a photo ID.
    let storage_root = (**state.storage_root.load()).clone();
    let now = Utc::now().to_rfc3339();

    let blob_row: Option<(String, String, i64, Option<String>, String, Option<String>)> =
        sqlx::query_as(
            "SELECT id, blob_type, size_bytes, client_hash, storage_path, content_hash \
             FROM blobs WHERE id = ? AND user_id = ?",
        )
        .bind(&req.blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?;

    // Track whether the source is a server-side photo (for cloning into photos table)
    let is_server_side = blob_row.is_none();

    /// Row shape for the full photos table query used when cloning server-side photos.
    #[derive(Debug, Clone, FromRow)]
    struct PhotoRowFull {
        filename: String,
        mime_type: String,
        media_type: String,
        file_path: String,
        size_bytes: i64,
        width: i32,
        height: i32,
        duration_secs: Option<f64>,
        taken_at: Option<String>,
        latitude: Option<f64>,
        longitude: Option<f64>,
        thumb_path: Option<String>,
        created_at: String,
        is_favorite: i32,
        crop_metadata: Option<String>,
        camera_model: Option<String>,
        photo_hash: Option<String>,
    }

    // Full photo row needed for server-side clones
    let photo_row_full: Option<PhotoRowFull> = if is_server_side {
        sqlx::query_as::<_, PhotoRowFull>(
            "SELECT filename, mime_type, media_type, file_path, size_bytes, width, height, \
                 duration_secs, taken_at, latitude, longitude, thumb_path, created_at, \
                 is_favorite, crop_metadata, camera_model, photo_hash \
                 FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&req.blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?
    } else {
        None
    };

    // Resolve source file path, metadata, and determine blob_type
    let (blob_type, size_bytes, client_hash, storage_path, content_hash): (
        String,
        i64,
        Option<String>,
        String,
        Option<String>,
    ) = if let Some((_id, bt, sz, ch, sp, coh)) = blob_row {
        (bt, sz, ch, sp, coh)
    } else {
        // Not in blobs table — use the photos table row
        let prf = photo_row_full
            .as_ref()
            .ok_or_else(|| AppError::BadRequest("Photo or blob not found".into()))?;

        // Derive blob_type from media_type (same logic as restore)
        let bt = match prf.media_type.as_str() {
            "gif" => "gif".to_string(),
            "video" => "video".to_string(),
            "audio" => "audio".to_string(),
            _ if prf.mime_type.starts_with("video/") => "video".to_string(),
            _ => "photo".to_string(),
        };
        (
            bt,
            prf.size_bytes,
            None,
            prf.file_path.clone(),
            prf.photo_hash.clone(),
        )
    };

    // Clone: read the original file data from disk, write a new copy under a fresh ID
    let blob_data = crate::blobs::storage::read_blob(&storage_root, &storage_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read source blob: {}", e)))?;

    let new_blob_id = Uuid::new_v4().to_string();
    let new_storage_path =
        crate::blobs::storage::write_blob(&storage_root, &auth.user_id, &new_blob_id, &blob_data)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write cloned blob: {}", e)))?;

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

    // For server-side (autoscanned) photos, also create a `photos` table row
    // for the clone.  This ensures the viewer's `/api/photos/{id}/file` and
    // `/api/photos/{id}/thumbnail` endpoints can serve the cloned file.
    if let Some(prf) = &photo_row_full {
        // Resolve the thumbnail: copy the original thumbnail file if it exists
        let new_thumb_path = if let Some(tp) = &prf.thumb_path {
            let thumb_data = crate::blobs::storage::read_blob(&storage_root, tp)
                .await
                .ok(); // Non-fatal if thumbnail missing
            if let Some(td) = thumb_data {
                let thumb_id = format!("{}_thumb", new_blob_id);
                crate::blobs::storage::write_blob(&storage_root, &auth.user_id, &thumb_id, &td)
                    .await
                    .ok()
            } else {
                None
            }
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
             thumb_path, created_at, is_favorite, crop_metadata, camera_model, photo_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&new_blob_id)
        .bind(&auth.user_id)
        .bind(&prf.filename)
        .bind(&new_storage_path)
        .bind(&prf.mime_type)
        .bind(&prf.media_type)
        .bind(prf.size_bytes)
        .bind(prf.width)
        .bind(prf.height)
        .bind(prf.duration_secs)
        .bind(&prf.taken_at)
        .bind(prf.latitude)
        .bind(prf.longitude)
        .bind(&new_thumb_path)
        .bind(&prf.created_at)
        .bind(prf.is_favorite)
        .bind(&prf.crop_metadata)
        .bind(&prf.camera_model)
        .bind(Option::<String>::None) // Don't copy photo_hash — it has a unique index per user
        .execute(&state.pool)
        .await?;

        tracing::info!(
            new_blob_id = %new_blob_id,
            original_id = %req.blob_id,
            mime_type = %prf.mime_type,
            "[DIAG:SECURE_ADD] Created photos table row for server-side clone"
        );
    }

    let item_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT OR IGNORE INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at, original_blob_id) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&item_id)
    .bind(&gallery_id)
    .bind(&new_blob_id)
    .bind(&now)
    .bind(&req.blob_id) // Track the original so it can be hidden from the main gallery
    .execute(&state.pool)
    .await?;

    tracing::info!(
        gallery_id = %gallery_id,
        original_blob_id = %req.blob_id,
        new_blob_id = %new_blob_id,
        item_id = %item_id,
        blob_type = %blob_type,
        is_server_side = is_server_side,
        "[DIAG:SECURE_ADD] Cloned blob into secure gallery"
    );

    // Trigger encryption migration for the newly created clone so the
    // EncryptionBanner doesn't report it as "pending" indefinitely.
    // Fire-and-forget — the response returns immediately.
    if is_server_side {
        let pool = state.pool.clone();
        let sr = (**state.storage_root.load()).clone();
        let jwt = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            crate::photos::server_migrate::auto_migrate_after_scan(pool, sr, jwt).await;
        });
    }

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
    // Return BOTH the cloned blob IDs and the original blob IDs so the
    // main gallery can hide originals that have been moved to secure albums.
    // Also include encrypted_blob_id and encrypted_thumb_blob_id of photos
    // in secure galleries so the web client can filter those from blob listings.
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT gi.blob_id, gi.original_blob_id \
         FROM encrypted_gallery_items gi \
         JOIN encrypted_galleries g ON g.id = gi.gallery_id \
         WHERE g.user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let mut ids = std::collections::HashSet::new();
    for (cloned_id, original_id) in &rows {
        ids.insert(cloned_id.clone());
        if let Some(orig) = original_id {
            ids.insert(orig.clone());
        }
    }

    // Also include encrypted_blob_id and encrypted_thumb_blob_id of photos
    // that are in secure galleries.  These blobs are created by server-side
    // encryption migration and have different IDs from the photos.id entries.
    let enc_blob_rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT p.encrypted_blob_id, p.encrypted_thumb_blob_id \
         FROM photos p \
         WHERE (p.id IN (SELECT gi.blob_id FROM encrypted_gallery_items gi \
                         JOIN encrypted_galleries g ON g.id = gi.gallery_id \
                         WHERE g.user_id = ?) \
                OR p.id IN (SELECT gi.original_blob_id FROM encrypted_gallery_items gi \
                            JOIN encrypted_galleries g ON g.id = gi.gallery_id \
                            WHERE g.user_id = ? AND gi.original_blob_id IS NOT NULL)) \
         AND (p.encrypted_blob_id IS NOT NULL OR p.encrypted_thumb_blob_id IS NOT NULL)",
    )
    .bind(&auth.user_id)
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    for (enc_blob, enc_thumb) in &enc_blob_rows {
        if let Some(eb) = enc_blob {
            ids.insert(eb.clone());
        }
        if let Some(et) = enc_thumb {
            ids.insert(et.clone());
        }
    }

    let id_vec: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();

    tracing::debug!(
        user_id = %auth.user_id,
        total_ids = id_vec.len(),
        cloned_count = rows.len(),
        "[DIAG:SECURE_IDS] Returning secure blob IDs (cloned + originals)"
    );

    Ok(Json(serde_json::json!({ "blob_ids": id_vec })))
}

/// GET /api/galleries/secure/:id/items
/// List items in a secure gallery (requires unlock token in header).
pub async fn list_gallery_items(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(gallery_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?")
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
        .ok_or_else(|| {
            AppError::Unauthorized("Gallery token required. Unlock the gallery first.".into())
        })?;

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
