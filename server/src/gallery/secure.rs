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
use crate::crypto;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use crate::photos::models::*;

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
/// (keyed-SHA256 signed, 1-hour TTL) that must be presented as
/// `X-Gallery-Token` to list a gallery's items.
///
/// The token is now verified server-side on the read path — see
/// [`crate::gallery::secure_token`] and [`list_gallery_items`].
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
        .map_err(|e| AppError::Internal(format!("Bcrypt verify failed: {e}")))?;

    if !valid {
        return Err(AppError::Unauthorized("Invalid password".into()));
    }

    let (token, expires_in) =
        crate::gallery::secure_token::generate(&auth.user_id, &state.config.auth.jwt_secret);

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

/// DELETE /api/galleries/secure/:gallery_id/items/:item_id
///
/// Remove an item from a secure gallery, returning the original photo to
/// the regular gallery.  This deletes the cloned blob (and clone photos
/// row, if any) created by `add_gallery_item`, and removes the
/// `encrypted_gallery_items` membership row.  The original photo —
/// referenced via `original_blob_id` — is automatically un-hidden the
/// next time the main gallery polls `/api/galleries/secure/blob-ids`.
///
/// Ownership of the gallery is verified before any deletion.
pub async fn remove_gallery_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((gallery_id, item_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    // Verify gallery ownership first.
    let owner: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?")
            .bind(&gallery_id)
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;
    if owner == 0 {
        return Err(AppError::NotFound);
    }

    // Look up the item — we need the cloned blob_id (and encrypted_*) to
    // delete the underlying files and DB rows.
    let item: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT blob_id, encrypted_blob_id, encrypted_thumb_blob_id \
         FROM encrypted_gallery_items WHERE id = ? AND gallery_id = ?",
    )
    .bind(&item_id)
    .bind(&gallery_id)
    .fetch_optional(&state.pool)
    .await?;

    let (clone_blob_id, enc_blob_id, enc_thumb_blob_id) = item.ok_or(AppError::NotFound)?;

    let storage_root = (**state.storage_root.load()).clone();

    // Delete the cloned blob file + row (if owned by this user).
    let clone_blob: Option<(String,)> =
        sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
            .bind(&clone_blob_id)
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await?;
    if let Some((sp,)) = clone_blob {
        let _ = crate::blobs::storage::delete_blob(&storage_root, &sp).await;
        let _ = sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
            .bind(&clone_blob_id)
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;
    }

    // Delete server-side clone photos row (and its thumbnail file) if any.
    // The clone uses the same id as the cloned blob.
    let clone_photo: Option<(String, Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT file_path, thumb_path, encrypted_blob_id, encrypted_thumb_blob_id \
         FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&clone_blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?;
    if let Some((fp, tp, photo_enc_blob, photo_enc_thumb)) = clone_photo {
        if !fp.is_empty() {
            let _ = crate::blobs::storage::delete_blob(&storage_root, &fp).await;
        }
        if let Some(tp) = tp {
            let _ = crate::blobs::storage::delete_blob(&storage_root, &tp).await;
        }
        // Delete encrypted blobs that belong only to this clone photo row
        for eb in [photo_enc_blob.as_deref(), photo_enc_thumb.as_deref()]
            .into_iter()
            .flatten()
        {
            if let Ok(Some((sp,))) = sqlx::query_as::<_, (String,)>(
                "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
            )
            .bind(eb)
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await
            {
                let _ = crate::blobs::storage::delete_blob(&storage_root, &sp).await;
                let _ = sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
                    .bind(eb)
                    .bind(&auth.user_id)
                    .execute(&state.pool)
                    .await;
            }
        }
        sqlx::query("DELETE FROM photos WHERE id = ? AND user_id = ?")
            .bind(&clone_blob_id)
            .bind(&auth.user_id)
            .execute(&state.pool)
            .await?;
    }

    // Delete encrypted_blob_id / encrypted_thumb_blob_id stored on the item
    // (used on backup servers when there is no photos clone row).  Avoid
    // double-deleting blobs we already removed above.
    for eb in [enc_blob_id.as_deref(), enc_thumb_blob_id.as_deref()]
        .into_iter()
        .flatten()
        .filter(|id| *id != clone_blob_id)
    {
        if let Ok(Some((sp,))) = sqlx::query_as::<_, (String,)>(
            "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
        )
        .bind(eb)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await
        {
            let _ = crate::blobs::storage::delete_blob(&storage_root, &sp).await;
            let _ = sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
                .bind(eb)
                .bind(&auth.user_id)
                .execute(&state.pool)
                .await;
        }
    }

    // Finally drop the membership row — the original photo becomes visible
    // again because `list_secure_blob_ids` will no longer include its id.
    sqlx::query("DELETE FROM encrypted_gallery_items WHERE id = ? AND gallery_id = ?")
        .bind(&item_id)
        .bind(&gallery_id)
        .execute(&state.pool)
        .await?;

    tracing::info!(
        gallery_id = %gallery_id,
        item_id = %item_id,
        clone_blob_id = %clone_blob_id,
        "[DIAG:SECURE_REMOVE] Removed item from secure gallery; original returned to gallery"
    );

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
        encrypted_blob_id: Option<String>,
    }

    // Full photo row needed for server-side clones
    let photo_row_full: Option<PhotoRowFull> = if is_server_side {
        sqlx::query_as::<_, PhotoRowFull>(
            "SELECT filename, mime_type, media_type, file_path, size_bytes, width, height, \
                 duration_secs, taken_at, latitude, longitude, thumb_path, created_at, \
                 is_favorite, crop_metadata, camera_model, photo_hash, encrypted_blob_id \
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
    let (blob_type, size_bytes, client_hash, storage_path, _content_hash): (
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

    // Clone: read the original file data from disk, write a new copy under a fresh ID.
    // For photos that have been server-side encrypted (empty file_path), decrypt
    // the encrypted blob to get the plaintext data.
    let blob_data = if storage_path.is_empty() {
        // Encrypted photo — decrypt from encrypted_blob_id
        let enc_blob_id = photo_row_full
            .as_ref()
            .and_then(|p| p.encrypted_blob_id.as_deref())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::BadRequest("Photo has no file on disk and no encrypted blob".into())
            })?;

        let enc_key = crypto::load_wrapped_key(&state.pool, &state.config.auth.jwt_secret)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to load encryption key: {e}")))?;
        let enc_key =
            enc_key.ok_or_else(|| AppError::Internal("No encryption key configured".into()))?;

        let enc_sp: Option<(String,)> =
            sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
                .bind(enc_blob_id)
                .bind(&auth.user_id)
                .fetch_optional(&state.pool)
                .await?;
        let (enc_storage_path,) = enc_sp
            .ok_or_else(|| AppError::Internal(format!("Encrypted blob {enc_blob_id} not found")))?;

        let enc_data = crate::blobs::storage::read_blob(&storage_root, &enc_storage_path).await?;
        let plaintext = {
            let k = enc_key;
            tokio::task::spawn_blocking(move || crypto::decrypt(&k, &enc_data))
                .await
                .map_err(|e| AppError::Internal(format!("Decrypt panicked: {e}")))?
                .map_err(|e| AppError::Internal(format!("Decrypt failed: {e}")))?
        };

        // Parse JSON envelope and extract the base64 "data" field
        let envelope: serde_json::Value = serde_json::from_slice(&plaintext)
            .map_err(|e| AppError::Internal(format!("Invalid blob envelope JSON: {e}")))?;
        let data_b64 = envelope["data"]
            .as_str()
            .ok_or_else(|| AppError::Internal("Missing 'data' field in blob envelope".into()))?;
        let raw_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data_b64)
                .map_err(|e| AppError::Internal(format!("Base64 decode failed: {e}")))?;

        tracing::info!(
            encrypted_blob_id = %enc_blob_id,
            decrypted_size = raw_bytes.len(),
            "[DIAG:SECURE_ADD] Decrypted encrypted photo for secure gallery clone"
        );

        raw_bytes
    } else {
        crate::blobs::storage::read_blob(&storage_root, &storage_path)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read source blob: {e}")))?
    };

    let new_blob_id = Uuid::new_v4().to_string();
    let new_storage_path =
        crate::blobs::storage::write_blob(&storage_root, &auth.user_id, &new_blob_id, &blob_data)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write cloned blob: {e}")))?;

    // Insert the cloned blob record.
    // content_hash is deliberately set to NULL so the server-side encryption
    // migration's dedup check does NOT match this plaintext clone blob.
    // Without this, the dedup incorrectly "reuses" the clone's own blob as
    // the encrypted_blob_id (pointing to unencrypted data → AES/GCM errors).
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&new_blob_id)
    .bind(&auth.user_id)
    .bind(&blob_type)
    .bind(size_bytes)
    .bind(&client_hash)
    .bind(&now)
    .bind(&new_storage_path)
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
                let thumb_id = format!("{new_blob_id}_thumb");
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

    // When the client sends an encrypted blob ID (Android: serverBlobId),
    // resolve the owning photo so we can store the **photo ID** as
    // original_blob_id.  This is critical because encrypted_sync hides
    // photos by `photos.id NOT IN (original_blob_id)`, and the blob
    // ID differs from the photo ID.
    let (resolved_original_id, original_enc_thumb): (String, Option<String>) = if !is_server_side {
        // The req.blob_id is a blobs-table ID.  Find the photo that owns it.
        let owner: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, photo_hash, encrypted_thumb_blob_id \
             FROM photos WHERE encrypted_blob_id = ? AND user_id = ?",
        )
        .bind(&req.blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?;

        if let Some((photo_id, _hash, thumb)) = owner {
            (photo_id, thumb)
        } else {
            // No owning photo found — fall back to blob ID
            (req.blob_id.clone(), None)
        }
    } else {
        (req.blob_id.clone(), None)
    };

    // Store the original photo's content hash so autoscan (run after recovery)
    // can skip files whose content matches a gallery-hidden original — even if
    // the file has been renamed or moved.
    let original_photo_hash: Option<String> = if is_server_side {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT photo_hash FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&req.blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .flatten()
    } else {
        // Client-encrypted blob — use the resolved photo's hash
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT photo_hash FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&resolved_original_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .flatten()
    };

    sqlx::query(
        "INSERT OR IGNORE INTO encrypted_gallery_items \
         (id, gallery_id, blob_id, added_at, original_blob_id, original_photo_hash, encrypted_blob_id, encrypted_thumb_blob_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&item_id)
    .bind(&gallery_id)
    .bind(&new_blob_id)
    .bind(&now)
    .bind(&resolved_original_id) // Photo ID — so encrypted_sync can hide the original
    .bind(&original_photo_hash)
    .bind(if !is_server_side { Some(&new_blob_id) } else { None::<&String> }) // Clone of encrypted data is already "encrypted"
    .bind(&original_enc_thumb) // Copy the original photo's encrypted thumb
    .execute(&state.pool)
    .await?;

    tracing::info!(
        gallery_id = %gallery_id,
        original_blob_id = %resolved_original_id,
        req_blob_id = %req.blob_id,
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

    // Also include encrypted_blob_id and encrypted_thumb_blob_id stored
    // directly on encrypted_gallery_items (populated on backup servers by
    // gallery metadata sync).  On the primary these columns are typically
    // NULL, but on backup the clone photos row may not exist in the photos
    // table, so the JOIN above would miss them.
    let egi_enc_rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT gi.encrypted_blob_id, gi.encrypted_thumb_blob_id \
         FROM encrypted_gallery_items gi \
         JOIN encrypted_galleries g ON g.id = gi.gallery_id \
         WHERE g.user_id = ? \
         AND (gi.encrypted_blob_id IS NOT NULL OR gi.encrypted_thumb_blob_id IS NOT NULL)",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    for (enc_blob, enc_thumb) in &egi_enc_rows {
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

    // Verify the unlock token: it must be a non-expired, correctly-signed
    // token issued to *this* user. Previously any non-empty string was
    // accepted, which made the password gate cosmetic.
    let token = headers
        .get("x-gallery-token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::Unauthorized("Gallery token required. Unlock the gallery first.".into())
        })?;

    if !crate::gallery::secure_token::verify(token, &auth.user_id, &state.config.auth.jwt_secret) {
        return Err(AppError::Unauthorized(
            "Invalid or expired gallery token. Unlock the gallery again.".into(),
        ));
    }

    // The subtype/burst/duration/motion fields live on the ORIGINAL photo
    // (`op`, joined via original_blob_id) — `add_gallery_item` does not copy
    // them onto the server-side clone row (`p`), so COALESCE falls through to
    // `op`. These let the Android secure viewer render videos, panoramas/360,
    // motion (LIVE) photos and collapse bursts the same way the main gallery
    // does.
    #[derive(FromRow)]
    struct GalleryItemRow {
        id: String,
        blob_id: String,
        added_at: String,
        encrypted_thumb_blob_id: Option<String>,
        width: Option<i64>,
        height: Option<i64>,
        media_type: Option<String>,
        photo_subtype: Option<String>,
        burst_id: Option<String>,
        duration_secs: Option<f64>,
        motion_video_blob_id: Option<String>,
    }

    let items = sqlx::query_as::<_, GalleryItemRow>(
        "SELECT gi.id, \
                COALESCE(gi.encrypted_blob_id, p.encrypted_blob_id, gi.blob_id) as blob_id, \
                gi.added_at, \
                COALESCE(gi.encrypted_thumb_blob_id, p.encrypted_thumb_blob_id, op.encrypted_thumb_blob_id) as encrypted_thumb_blob_id, \
                COALESCE(p.width, op.width) as width, \
                COALESCE(p.height, op.height) as height, \
                COALESCE(p.media_type, op.media_type) as media_type, \
                COALESCE(p.photo_subtype, op.photo_subtype) as photo_subtype, \
                COALESCE(p.burst_id, op.burst_id) as burst_id, \
                COALESCE(p.duration_secs, op.duration_secs) as duration_secs, \
                COALESCE(p.motion_video_blob_id, op.motion_video_blob_id) as motion_video_blob_id \
         FROM encrypted_gallery_items gi \
         LEFT JOIN photos p ON p.id = gi.blob_id AND p.encrypted_blob_id IS NOT NULL \
         LEFT JOIN photos op ON op.id = gi.original_blob_id \
         WHERE gi.gallery_id = ? \
         ORDER BY gi.added_at DESC",
    )
    .bind(&gallery_id)
    .fetch_all(&state.pool)
    .await?;

    let items_json: Vec<serde_json::Value> = items
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "blob_id": r.blob_id,
                "added_at": r.added_at,
                "encrypted_thumb_blob_id": r.encrypted_thumb_blob_id,
                "width": r.width,
                "height": r.height,
                "media_type": r.media_type,
                "photo_subtype": r.photo_subtype,
                "burst_id": r.burst_id,
                "duration_secs": r.duration_secs,
                "motion_video_blob_id": r.motion_video_blob_id,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "items": items_json })))
}
