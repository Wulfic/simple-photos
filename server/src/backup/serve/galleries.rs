//! Secure-gallery sync endpoint: full-state replacement of
//! `encrypted_galleries` / `encrypted_gallery_items` plus a retroactive purge
//! of stale photos/blobs that have since become secure-album clones.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use crate::error::AppError;
use crate::state::AppState;

use super::validate_api_key;

/// POST /api/backup/sync-secure-galleries
/// Receives the full state of `encrypted_galleries` and `encrypted_gallery_items`
/// from the primary.  Upserts all rows and removes any that no longer exist on
/// the primary (full-state replacement).
///
/// This ensures the backup knows which `photos` rows are secure-album clones
/// and can filter them from the regular gallery view via `encrypted-sync`.
pub async fn backup_sync_secure_galleries(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let galleries = body
        .get("galleries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut tx = state.pool.begin().await?;

    // Upsert galleries
    let mut gallery_ids = std::collections::HashSet::new();
    for g in &galleries {
        let id = g["id"].as_str().unwrap_or_default();
        let user_id = g["user_id"].as_str().unwrap_or_default();
        let name = g["name"].as_str().unwrap_or_default();
        let password_hash = g["password_hash"].as_str().unwrap_or("account-auth");
        let created_at = g["created_at"].as_str().unwrap_or_default();

        if id.is_empty() || user_id.is_empty() {
            continue;
        }

        gallery_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO encrypted_galleries (id, user_id, name, password_hash, created_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name, \
               password_hash = excluded.password_hash",
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(password_hash)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }

    // Upsert items
    let mut item_ids = std::collections::HashSet::new();
    for i in &items {
        let id = i["id"].as_str().unwrap_or_default();
        let gallery_id = i["gallery_id"].as_str().unwrap_or_default();
        let blob_id = i["blob_id"].as_str().unwrap_or_default();
        let added_at = i["added_at"].as_str().unwrap_or_default();
        let original_blob_id = i["original_blob_id"].as_str();
        let encrypted_blob_id = i["encrypted_blob_id"].as_str();
        let encrypted_thumb_blob_id = i["encrypted_thumb_blob_id"].as_str();
        let original_photo_hash = i["original_photo_hash"].as_str();

        if id.is_empty() || gallery_id.is_empty() || blob_id.is_empty() {
            continue;
        }

        item_ids.insert(id.to_string());

        // The clone blob_id may not exist on the backup (clones are excluded
        // from blob sync).  Insert a placeholder row to satisfy the FK
        // constraint on encrypted_gallery_items.blob_id → blobs(id).
        // Only the metadata matters — the actual encrypted data stays on the
        // primary and is never served from the backup.
        let blob_exists: bool =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM blobs WHERE id = ?)")
                .bind(blob_id)
                .fetch_one(&mut *tx)
                .await
                .unwrap_or(false);

        if !blob_exists {
            // Resolve a valid user_id for the FK on blobs.user_id.
            let admin_uid: String = sqlx::query_scalar(
                "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
            )
            .fetch_optional(&mut *tx)
            .await
            .unwrap_or(None)
            .unwrap_or_default();

            let _ = sqlx::query(
                "INSERT OR IGNORE INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
                 VALUES (?, ?, 'gallery-placeholder', 0, '', '')",
            )
            .bind(blob_id)
            .bind(&admin_uid)
            .execute(&mut *tx)
            .await;
        }

        sqlx::query(
            "INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at, original_blob_id, encrypted_blob_id, encrypted_thumb_blob_id, original_photo_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               blob_id = excluded.blob_id, \
               original_blob_id = excluded.original_blob_id, \
               encrypted_blob_id = excluded.encrypted_blob_id, \
               encrypted_thumb_blob_id = excluded.encrypted_thumb_blob_id, \
               original_photo_hash = excluded.original_photo_hash",
        )
        .bind(id)
        .bind(gallery_id)
        .bind(blob_id)
        .bind(added_at)
        .bind(original_blob_id)
        .bind(encrypted_blob_id)
        .bind(encrypted_thumb_blob_id)
        .bind(original_photo_hash)
        .execute(&mut *tx)
        .await?;
    }

    // Remove galleries/items that no longer exist on the primary.
    // Only prune if the primary sent at least one gallery (avoid wiping
    // everything when the request is empty due to a transient error).
    if !gallery_ids.is_empty() {
        let existing_gallery_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM encrypted_galleries")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();

        for existing_id in &existing_gallery_ids {
            if !gallery_ids.contains(existing_id) {
                sqlx::query("DELETE FROM encrypted_galleries WHERE id = ?")
                    .bind(existing_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    if !item_ids.is_empty() {
        let existing_item_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM encrypted_gallery_items")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();

        for existing_id in &existing_item_ids {
            if !item_ids.contains(existing_id) {
                sqlx::query("DELETE FROM encrypted_gallery_items WHERE id = ?")
                    .bind(existing_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    // ── Retroactive purge ────────────────────────────────────────────────
    // Items that were synced to this backup BEFORE being added to a secure
    // gallery on the primary still exist in our photos/blobs tables.  Now
    // that we have the authoritative gallery item list, remove any stale
    // rows so they no longer appear in backup listings.
    //
    // Collect all IDs that should be hidden: clone blob_ids and the
    // original_blob_ids they shadow.
    let hidden_ids: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT blob_id FROM encrypted_gallery_items \
         UNION \
         SELECT original_blob_id FROM encrypted_gallery_items \
         WHERE original_blob_id IS NOT NULL",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap_or_default();

    let mut purged_photos = 0usize;
    let mut purged_blobs = 0usize;
    // Collect file paths to delete from disk AFTER the transaction commits.
    // This prevents autoscan from re-registering purged gallery items.
    let mut files_to_delete: Vec<String> = Vec::new();

    for hidden_id in &hidden_ids {
        // Collect file paths and thumb paths BEFORE deleting the photo row
        let paths: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT file_path, thumb_path FROM photos WHERE id = ?")
                .bind(hidden_id)
                .fetch_optional(&mut *tx)
                .await
                .unwrap_or(None);
        if let Some((ref fp, ref tp)) = paths {
            if let Some(ref p) = fp {
                files_to_delete.push(p.clone());
            }
            if let Some(ref p) = tp {
                files_to_delete.push(p.clone());
            }
        }

        // Also collect storage_path for blob rows we're about to delete
        let blob_path: Option<String> = sqlx::query_scalar(
            "SELECT storage_path FROM blobs WHERE id = ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items)",
        )
        .bind(hidden_id)
        .fetch_optional(&mut *tx)
        .await
        .unwrap_or(None);
        if let Some(ref bp) = blob_path {
            files_to_delete.push(bp.clone());
        }

        // Remove server-side encryption blobs linked to the photo row
        // before deleting the photo itself (avoids orphaned blob rows).
        let enc_ids: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT encrypted_blob_id, encrypted_thumb_blob_id \
             FROM photos WHERE id = ?",
        )
        .bind(hidden_id)
        .fetch_optional(&mut *tx)
        .await
        .unwrap_or(None);

        if let Some((ref enc_blob, ref enc_thumb)) = enc_ids {
            if let Some(ref bid) = enc_blob {
                // Guard: do NOT delete this blob if a gallery item references
                // it via encrypted_blob_id or encrypted_thumb_blob_id.
                // The backup_receive_blob dedup logic can reassign the
                // original photo's encrypted_blob_id to point to the synced
                // clone's encrypted blob (same content_hash).  Without this
                // guard, the purge would delete the clone's encrypted blob
                // that the gallery needs → 404 on backup.
                let is_gallery_ref: bool = sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM encrypted_gallery_items \
                     WHERE encrypted_blob_id = ? OR encrypted_thumb_blob_id = ?)",
                )
                .bind(bid)
                .bind(bid)
                .fetch_one(&mut *tx)
                .await
                .unwrap_or(true);

                if !is_gallery_ref {
                    let enc_path: Option<String> =
                        sqlx::query_scalar("SELECT storage_path FROM blobs WHERE id = ?")
                            .bind(bid)
                            .fetch_optional(&mut *tx)
                            .await
                            .unwrap_or(None);
                    if let Some(ref ep) = enc_path {
                        files_to_delete.push(ep.clone());
                    }
                    let _ = sqlx::query("DELETE FROM blobs WHERE id = ?")
                        .bind(bid)
                        .execute(&mut *tx)
                        .await;
                } else {
                    tracing::info!(
                        "Purge: skipping encrypted blob {} (referenced by gallery item)",
                        bid
                    );
                }
            }
            if let Some(ref tid) = enc_thumb {
                // Same guard for encrypted thumbnail blobs.
                let is_gallery_ref: bool = sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM encrypted_gallery_items \
                     WHERE encrypted_blob_id = ? OR encrypted_thumb_blob_id = ?)",
                )
                .bind(tid)
                .bind(tid)
                .fetch_one(&mut *tx)
                .await
                .unwrap_or(true);

                if !is_gallery_ref {
                    let enc_thumb_path: Option<String> =
                        sqlx::query_scalar("SELECT storage_path FROM blobs WHERE id = ?")
                            .bind(tid)
                            .fetch_optional(&mut *tx)
                            .await
                            .unwrap_or(None);
                    if let Some(ref etp) = enc_thumb_path {
                        files_to_delete.push(etp.clone());
                    }
                    let _ = sqlx::query("DELETE FROM blobs WHERE id = ?")
                        .bind(tid)
                        .execute(&mut *tx)
                        .await;
                } else {
                    tracing::info!(
                        "Purge: skipping encrypted thumb blob {} (referenced by gallery item)",
                        tid
                    );
                }
            }
        }

        // Remove from photos table (covers pre-synced originals + clones)
        let r = sqlx::query("DELETE FROM photos WHERE id = ?")
            .bind(hidden_id)
            .execute(&mut *tx)
            .await;
        if let Ok(ref res) = r {
            purged_photos += res.rows_affected() as usize;
        }

        // Clean up orphaned photo_tags
        let _ = sqlx::query("DELETE FROM photo_tags WHERE photo_id = ?")
            .bind(hidden_id)
            .execute(&mut *tx)
            .await;

        // Remove from blobs table (covers pre-synced client-encrypted blobs)
        // BUT skip blobs that are referenced by encrypted_gallery_items.blob_id
        // because those FK rows need to stay (ON DELETE CASCADE would destroy
        // the gallery item we just synced).
        let r = sqlx::query(
            "DELETE FROM blobs WHERE id = ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items)",
        )
        .bind(hidden_id)
        .execute(&mut *tx)
        .await;
        if let Ok(ref res) = r {
            purged_blobs += res.rows_affected() as usize;
        }
    }

    // Acquire scan_lock BEFORE committing so that a concurrent autoscan
    // cannot re-register a purged file in the window between the DB commit
    // (which removes the photos row) and the physical file deletion.
    // background_auto_scan_task uses try_lock(), so it will simply skip
    // this cycle rather than block.
    let _scan_guard = state.scan_lock.lock().await;

    tx.commit().await?;

    // Delete physical files from disk so autoscan cannot re-register them.
    // The scan_lock is held, preventing any concurrent scan from seeing
    // orphaned files between the DB purge and physical deletion.
    if !files_to_delete.is_empty() {
        let storage_root = (**state.storage_root.load()).clone();
        let mut deleted_files = 0usize;
        for rel_path in &files_to_delete {
            let full_path = storage_root.join(rel_path);
            match tokio::fs::remove_file(&full_path).await {
                Ok(_) => {
                    deleted_files += 1;
                    tracing::debug!("Purged file from disk: {}", rel_path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Already gone — not an error
                }
                Err(e) => {
                    tracing::warn!("Failed to delete purged file {}: {}", rel_path, e);
                }
            }
        }
        if deleted_files > 0 {
            tracing::info!(
                "Secure gallery purge: deleted {} physical files from disk",
                deleted_files
            );
        }
    }

    drop(_scan_guard);

    if purged_photos > 0 || purged_blobs > 0 {
        tracing::info!(
            "Secure gallery sync: purged {} stale photos and {} stale blobs from backup",
            purged_photos,
            purged_blobs
        );
    }

    tracing::info!(
        "Received secure gallery sync: {} galleries, {} items",
        galleries.len(),
        items.len()
    );

    Ok(StatusCode::OK)
}
