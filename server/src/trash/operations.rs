//! Trash mutation operations: soft-delete, restore, permanent-delete, empty.
//!
//! All multi-step DB operations are wrapped in SQLite transactions for
//! atomicity. Backup-mode servers allow admins to manage content owned
//! by synced user stubs.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use super::models::*;

// ── Soft-delete (unencrypted photo) ──────────────────────────────────────────

/// DELETE /api/photos/:id
/// Soft-delete an unencrypted photo to the trash. All metadata is read from the
/// photos table so the client doesn't need to supply anything.
pub async fn soft_delete_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if Uuid::parse_str(&photo_id).is_err() {
        return Err(AppError::BadRequest("Invalid photo ID format".into()));
    }

    let mut tx = state.pool.begin().await?;

    // Fetch the photo row in two queries to stay within SQLx tuple size limits
    let photo_core = sqlx::query_as::<_, (
        String, String, String, String, i64, i64, i64,
        Option<f64>, Option<String>,
    )>(
        "SELECT filename, file_path, mime_type, media_type, size_bytes, width, height, \
         duration_secs, taken_at \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    let photo_extra = sqlx::query_as::<_, (
        Option<f64>, Option<f64>, Option<String>, i64,
        Option<String>, Option<String>, Option<String>,
    )>(
        "SELECT latitude, longitude, thumb_path, is_favorite, \
         crop_metadata, camera_model, photo_hash \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_one(&mut *tx)
    .await?;

    let retention_days: i64 = sqlx::query_scalar(
        "SELECT CAST(value AS INTEGER) FROM server_settings WHERE key = 'trash_retention_days'",
    )
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(30);

    let now = Utc::now();
    let expires_at = now + Duration::days(retention_days);
    let trash_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO trash_items (id, user_id, photo_id, filename, file_path, mime_type, \
         media_type, size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
         thumb_path, deleted_at, expires_at, is_favorite, crop_metadata, camera_model, photo_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .bind(&photo_id)
    .bind(&photo_core.0)  // filename
    .bind(&photo_core.1)  // file_path
    .bind(&photo_core.2)  // mime_type
    .bind(&photo_core.3)  // media_type
    .bind(photo_core.4)   // size_bytes
    .bind(photo_core.5)   // width
    .bind(photo_core.6)   // height
    .bind(photo_core.7)   // duration_secs
    .bind(&photo_core.8)  // taken_at
    .bind(photo_extra.0)  // latitude
    .bind(photo_extra.1)  // longitude
    .bind(&photo_extra.2) // thumb_path
    .bind(now.to_rfc3339())
    .bind(expires_at.to_rfc3339())
    .bind(photo_extra.3)  // is_favorite
    .bind(&photo_extra.4) // crop_metadata
    .bind(&photo_extra.5) // camera_model
    .bind(&photo_extra.6) // photo_hash
    .execute(&mut *tx)
    .await?;

    // Remove from photos table (keep files on disk for restore)
    sqlx::query("DELETE FROM photos WHERE id = ? AND user_id = ?")
        .bind(&photo_id)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    // Clean up shared album references
    sqlx::query("DELETE FROM shared_album_photos WHERE photo_ref = ?")
        .bind(&photo_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    audit::log(
        &state,
        AuditEvent::TrashSoftDelete,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "photo_id": photo_id,
            "trash_id": trash_id,
            "filename": photo_core.0,
            "expires_at": expires_at.to_rfc3339(),
        })),
    )
    .await;

    Ok(Json(serde_json::json!({
        "trash_id": trash_id,
        "expires_at": expires_at.to_rfc3339(),
    })))
}

// ── Soft-delete (encrypted blob) ─────────────────────────────────────────────

/// POST /api/blobs/:id/trash
/// Soft-delete an encrypted blob to the trash. The client provides the metadata
/// since the server stores blobs opaquely.
pub async fn soft_delete_blob(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(blob_id): Path<String>,
    Json(req): Json<SoftDeleteBlobRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate blob_id format
    if Uuid::parse_str(&blob_id).is_err() {
        return Err(AppError::BadRequest("Invalid blob ID format".into()));
    }

    // Sanitize client-supplied metadata before starting the transaction
    let safe_filename = sanitize::sanitize_filename(&req.filename);
    let safe_mime = sanitize::sanitize_freeform(&req.mime_type, 128);
    let media_type = req.media_type.as_deref().unwrap_or("photo");
    let size_bytes = req.size_bytes.unwrap_or(0);

    // Begin transaction — INSERT trash + DELETE blob(s) must be atomic
    let mut tx = state.pool.begin().await?;

    // Fetch the blob record (storage_path + hashes for preservation)
    let blob_row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT storage_path, client_hash, content_hash FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await?;

    // On backup servers, blobs may be owned by a synced user stub whose ID
    // differs from the logged-in admin.  Allow admin users to manage all content.
    let (storage_path, client_hash, content_hash, blob_owner_id) = if let Some(row) = blob_row {
        (row.0, row.1, row.2, auth.user_id.clone())
    } else if is_backup_mode(&state.read_pool).await
        && is_admin_user(&state.read_pool, &auth.user_id).await
    {
        let row = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
            "SELECT user_id, storage_path, client_hash, content_hash FROM blobs WHERE id = ?",
        )
        .bind(&blob_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
        (row.1, row.2, row.3, row.0)
    } else {
        return Err(AppError::NotFound);
    };

    // Optionally fetch thumbnail blob storage_path
    let thumb_storage_path = if let Some(ref thumb_id) = req.thumbnail_blob_id {
        // Try with auth user first, then with blob owner
        let result = sqlx::query_scalar::<_, String>(
            "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
        )
        .bind(thumb_id)
        .bind(&blob_owner_id)
        .fetch_optional(&mut *tx)
        .await?;
        result
    } else {
        None
    };

    // Read retention days from server_settings (default 30)
    let retention_days: i64 = sqlx::query_scalar(
        "SELECT CAST(value AS INTEGER) FROM server_settings WHERE key = 'trash_retention_days'",
    )
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(30);

    let now = Utc::now();
    let expires_at = now + Duration::days(retention_days);
    let trash_id = Uuid::new_v4().to_string();

    // Insert into trash_items with blob references and hash preservation
    sqlx::query(
        "INSERT INTO trash_items (id, user_id, photo_id, filename, file_path, mime_type, \
         media_type, size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
         thumb_path, deleted_at, expires_at, encrypted_blob_id, thumbnail_blob_id, \
         client_hash, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&trash_id)
    .bind(&blob_owner_id)
    .bind(&blob_id) // photo_id = blob_id for encrypted items
    .bind(&safe_filename)
    .bind(&storage_path) // file_path = blob storage_path
    .bind(&safe_mime)
    .bind(media_type)
    .bind(size_bytes)
    .bind(req.width.unwrap_or(0))
    .bind(req.height.unwrap_or(0))
    .bind(req.duration_secs)
    .bind(&req.taken_at)
    .bind(&thumb_storage_path) // thumb_path = thumbnail blob storage_path
    .bind(now.to_rfc3339())
    .bind(expires_at.to_rfc3339())
    .bind(&blob_id)
    .bind(&req.thumbnail_blob_id)
    .bind(&client_hash)
    .bind(&content_hash)
    .execute(&mut *tx)
    .await?;

    // Remove blob from blobs table (but keep files on disk!)
    sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
        .bind(&blob_id)
        .bind(&blob_owner_id)
        .execute(&mut *tx)
        .await?;

    // Also remove thumbnail blob record if present
    if let Some(ref thumb_id) = req.thumbnail_blob_id {
        sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
            .bind(thumb_id)
            .bind(&blob_owner_id)
            .execute(&mut *tx)
            .await?;
    }

    // Remove the photos-table row that links to this encrypted blob so that
    // the encrypted-sync endpoint stops returning the deleted item.  Without
    // this, loadEncryptedPhotos() would re-add the photo to the client's IDB
    // (without thumbnail data) on the very next sync, making it appear as
    // though the deletion never happened.
    sqlx::query("DELETE FROM photos WHERE encrypted_blob_id = ? AND user_id = ?")
        .bind(&blob_id)
        .bind(&blob_owner_id)
        .execute(&mut *tx)
        .await?;

    // Clean up shared album references to prevent dangling photo_ref entries
    sqlx::query("DELETE FROM shared_album_photos WHERE photo_ref = ? AND ref_type = 'blob'")
        .bind(&blob_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    audit::log(
        &state,
        AuditEvent::TrashSoftDelete,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "blob_id": blob_id,
            "trash_id": trash_id,
            "filename": safe_filename,
            "expires_at": expires_at.to_rfc3339(),
        })),
    )
    .await;

    tracing::info!(
        "Encrypted blob {} moved to trash (expires {})",
        blob_id,
        expires_at.to_rfc3339()
    );

    Ok(Json(serde_json::json!({
        "trash_id": trash_id,
        "expires_at": expires_at.to_rfc3339(),
    })))
}

// ── Restore ──────────────────────────────────────────────────────────────────

/// POST /api/trash/:id/restore
/// Restore an encrypted blob from the trash back to the blobs table.
pub async fn restore_from_trash(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(trash_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Begin transaction — all restore operations must be atomic
    let mut tx = state.pool.begin().await?;

    // Fetch the trash item. On backup servers, admins can manage all content.
    let row = sqlx::query_as::<_, TrashBlobRow>(
        "SELECT file_path, mime_type, media_type, size_bytes, thumb_path, \
         thumbnail_blob_id, client_hash, content_hash \
         FROM trash_items WHERE id = ? AND user_id = ?",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (row, owner_id) = match row {
        Some(r) => (r, auth.user_id.clone()),
        None => {
            if is_backup_mode(&state.read_pool).await
                && is_admin_user(&state.read_pool, &auth.user_id).await
            {
                let r = sqlx::query_as::<_, TrashBlobRow>(
                    "SELECT file_path, mime_type, media_type, size_bytes, thumb_path, \
                     thumbnail_blob_id, client_hash, content_hash \
                     FROM trash_items WHERE id = ?",
                )
                .bind(&trash_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(AppError::NotFound)?;

                let actual_owner: String = sqlx::query_scalar(
                    "SELECT user_id FROM trash_items WHERE id = ?",
                )
                .bind(&trash_id)
                .fetch_one(&mut *tx)
                .await?;

                (r, actual_owner)
            } else {
                return Err(AppError::NotFound);
            }
        }
    };

    // Fetch the encrypted blob ID. Items trashed via the encrypted path always
    // have this set. Items from the photo soft-delete path lack it.
    let encrypted_blob_id: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT encrypted_blob_id FROM trash_items WHERE id = ?",
    )
    .bind(&trash_id)
    .fetch_one(&mut *tx)
    .await?;

    if let Some(ref blob_id) = encrypted_blob_id {
        // ── Encrypted blob restore path ──────────────────────────────
        let blob_type = match row.media_type.as_str() {
            "gif" => "gif",
            "video" => "video",
            "audio" => "audio",
            _ => "photo",
        };

        let now = chrono::Utc::now().to_rfc3339();

        // Re-insert the main blob (restoring hash fields for dedup/integrity)
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path, \
             client_hash, content_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(blob_id)
        .bind(&owner_id)
        .bind(blob_type)
        .bind(row.size_bytes)
        .bind(&now)
        .bind(&row.file_path)
        .bind(&row.client_hash)
        .bind(&row.content_hash)
        .execute(&mut *tx)
        .await?;

        // Re-insert the thumbnail blob if present
        if let (Some(ref thumb_blob_id), Some(ref thumb_path)) =
            (&row.thumbnail_blob_id, &row.thumb_path)
        {
            sqlx::query(
                "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
                 VALUES (?, ?, 'thumbnail', 0, ?, ?)",
            )
            .bind(thumb_blob_id)
            .bind(&owner_id)
            .bind(&now)
            .bind(thumb_path)
            .execute(&mut *tx)
            .await?;
        }

        // Remove from trash
        sqlx::query("DELETE FROM trash_items WHERE id = ?")
            .bind(&trash_id)
            .execute(&mut *tx)
            .await?;

        tracing::info!("Encrypted blob {} restored from trash", blob_id);

        audit::log(
            &state,
            AuditEvent::TrashRestore,
            Some(&auth.user_id),
            &headers,
            Some(serde_json::json!({
                "trash_id": trash_id,
                "blob_id": blob_id,
            })),
        )
        .await;
    } else {
        // ── Unencrypted photo restore path ───────────────────────────
        // Fetch photo columns from trash in two queries to stay within tuple limits
        let photo_core = sqlx::query_as::<_, (
            String, String, String, String, String, i64, i64, i64,
            Option<f64>, Option<String>,
        )>(
            "SELECT photo_id, filename, file_path, mime_type, media_type, size_bytes, \
             width, height, duration_secs, taken_at \
             FROM trash_items WHERE id = ?",
        )
        .bind(&trash_id)
        .fetch_one(&mut *tx)
        .await?;

        let photo_extra = sqlx::query_as::<_, (
            Option<f64>, Option<f64>, Option<String>, i64,
            Option<String>, Option<String>, Option<String>,
        )>(
            "SELECT latitude, longitude, thumb_path, is_favorite, \
             crop_metadata, camera_model, photo_hash \
             FROM trash_items WHERE id = ?",
        )
        .bind(&trash_id)
        .fetch_one(&mut *tx)
        .await?;

        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
             thumb_path, created_at, is_favorite, crop_metadata, camera_model, photo_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&photo_core.0)    // photo_id → id
        .bind(&owner_id)
        .bind(&photo_core.1)    // filename
        .bind(&photo_core.2)    // file_path
        .bind(&photo_core.3)    // mime_type
        .bind(&photo_core.4)    // media_type
        .bind(photo_core.5)     // size_bytes
        .bind(photo_core.6)     // width
        .bind(photo_core.7)     // height
        .bind(photo_core.8)     // duration_secs
        .bind(&photo_core.9)    // taken_at
        .bind(photo_extra.0)    // latitude
        .bind(photo_extra.1)    // longitude
        .bind(&photo_extra.2)   // thumb_path
        .bind(&now)             // created_at
        .bind(photo_extra.3)    // is_favorite
        .bind(&photo_extra.4)   // crop_metadata
        .bind(&photo_extra.5)   // camera_model
        .bind(&photo_extra.6)   // photo_hash
        .execute(&mut *tx)
        .await?;

        // Remove from trash
        sqlx::query("DELETE FROM trash_items WHERE id = ?")
            .bind(&trash_id)
            .execute(&mut *tx)
            .await?;

        tracing::info!("Photo {} restored from trash", photo_core.0);

        audit::log(
            &state,
            AuditEvent::TrashRestore,
            Some(&auth.user_id),
            &headers,
            Some(serde_json::json!({
                "trash_id": trash_id,
                "photo_id": photo_core.0,
            })),
        )
        .await;
    }

    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Permanent delete ─────────────────────────────────────────────────────────

/// DELETE /api/trash/:id
/// Permanently delete a single item from the trash (and its files on disk).
pub async fn permanent_delete(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(trash_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Begin transaction — ref-count check + DELETE must be atomic to prevent TOCTOU races
    let mut tx = state.pool.begin().await?;

    let item: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT file_path, thumb_path FROM trash_items WHERE id = ? AND user_id = ?",
    )
    .bind(&trash_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (file_path, thumb_path) = item.ok_or(AppError::NotFound)?;

    // Only delete files from disk if no other photo row references the same
    // file_path (which happens when the user duplicates a photo via "Save Copy").
    let other_refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE file_path = ?")
        .bind(&file_path)
        .fetch_one(&mut *tx)
        .await?;

    let can_delete_file = other_refs == 0;

    let can_delete_thumb = if let Some(ref tp) = thumb_path {
        let other_thumb_refs: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE thumb_path = ?")
                .bind(tp)
                .fetch_one(&mut *tx)
                .await?;
        other_thumb_refs == 0
    } else {
        false
    };

    // Remove from database first (within the transaction)
    sqlx::query("DELETE FROM trash_items WHERE id = ? AND user_id = ?")
        .bind(&trash_id)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // Delete files from disk AFTER commit — a failure here is a minor storage
    // leak but preserves data integrity (the trash row is already gone).
    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();

    if can_delete_file {
        let full_path = storage_root.join(&file_path);
        if tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
            if let Err(e) = tokio::fs::remove_file(&full_path).await {
                tracing::warn!("Failed to delete photo file {}: {}", file_path, e);
            }
        }
    }

    if let Some(ref tp) = thumb_path {
        if can_delete_thumb {
            let thumb_full = storage_root.join(tp);
            if tokio::fs::try_exists(&thumb_full).await.unwrap_or(false) {
                if let Err(e) = tokio::fs::remove_file(&thumb_full).await {
                    tracing::warn!("Failed to delete thumbnail {}: {}", tp, e);
                }
            }
        }
    }

    tracing::info!("Permanently deleted trash item {}", trash_id);

    audit::log(
        &state,
        AuditEvent::TrashPermanentDelete,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "trash_id": trash_id,
        })),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

// ── Empty all ────────────────────────────────────────────────────────────────

/// DELETE /api/trash
/// Empty the entire trash (permanently delete all items for this user).
pub async fn empty_trash(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    // Begin transaction — ref-count checks + batch DELETE must be atomic
    let mut tx = state.pool.begin().await?;

    // Fetch all trash items for file cleanup
    let items: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT file_path, thumb_path FROM trash_items WHERE user_id = ?")
            .bind(&auth.user_id)
            .fetch_all(&mut *tx)
            .await?;

    let deleted_count = items.len() as i64;

    // Build a list of files safe to delete (no other photo row references them).
    // We check within the transaction to avoid TOCTOU races.
    let mut files_to_delete: Vec<std::path::PathBuf> = Vec::new();
    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();

    for (file_path, thumb_path) in &items {
        let other_refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE file_path = ?")
            .bind(file_path)
            .fetch_one(&mut *tx)
            .await?;

        if other_refs == 0 {
            files_to_delete.push(storage_root.join(file_path));
        }

        if let Some(tp) = thumb_path {
            let other_thumb_refs: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE thumb_path = ?")
                    .bind(tp)
                    .fetch_one(&mut *tx)
                    .await?;

            if other_thumb_refs == 0 {
                files_to_delete.push(storage_root.join(tp));
            }
        }
    }

    // Remove all rows from database (within the transaction)
    sqlx::query("DELETE FROM trash_items WHERE user_id = ?")
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // Delete files from disk AFTER commit — failures here are minor storage
    // leaks but preserve data integrity.
    for path in &files_to_delete {
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            if let Err(e) = tokio::fs::remove_file(path).await {
                tracing::warn!("Failed to delete file {:?}: {}", path, e);
            }
        }
    }

    tracing::info!(
        "Emptied trash for user {}: {} items permanently deleted",
        auth.user_id,
        deleted_count
    );

    audit::log(
        &state,
        AuditEvent::TrashEmpty,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "deleted_count": deleted_count,
        })),
    )
    .await;

    Ok(Json(serde_json::json!({
        "deleted": deleted_count,
        "message": format!("{} items permanently deleted", deleted_count),
    })))
}

// ── Internal helpers ────────────────────────────────────────────────────────

/// Check whether this server is running in backup mode.
async fn is_backup_mode(pool: &sqlx::SqlitePool) -> bool {
    sqlx::query_scalar::<_, String>("SELECT value FROM server_settings WHERE key = 'backup_mode'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("backup")
}

/// Check whether the authenticated user has the admin role.
async fn is_admin_user(pool: &sqlx::SqlitePool, user_id: &str) -> bool {
    sqlx::query_scalar::<_, String>("SELECT role FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("admin")
}

/// Row type for restoring encrypted blob items from trash.
/// Extended in migration 020 to preserve hash fields for dedup/integrity.
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct TrashBlobRow {
    file_path: String,
    mime_type: String,
    media_type: String,
    size_bytes: i64,
    thumb_path: Option<String>,
    thumbnail_blob_id: Option<String>,
    client_hash: Option<String>,
    content_hash: Option<String>,
}
