//! Deletion-sync endpoint: evict photos/blobs that have been trashed on the
//! primary so they no longer appear in the backup gallery.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use crate::error::AppError;
use crate::state::AppState;

use super::validate_api_key;

/// POST /api/backup/sync-deletions
/// Accepts a list of photo IDs that have been deleted on the primary server
/// (i.e., are now in the primary's trash) and removes them from the backup's
/// `photos` + `photo_tags` tables so the items no longer appear in the gallery.
///
/// This is called during every sync so that items deleted on the primary are
/// evicted from the backup gallery even when they were already in
/// `remote_trash_ids` (and therefore skipped by the file-transfer delta logic).
pub async fn backup_sync_deletions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let ids: Vec<String> = body
        .get("deleted_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let file_paths: Vec<String> = body
        .get("deleted_file_paths")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if ids.is_empty() && file_paths.is_empty() {
        return Ok(StatusCode::OK);
    }

    let mut removed = 0usize;
    for id in &ids {
        // Remove from gallery by id OR encrypted_blob_id — for encrypted
        // items the trash stores blob_id as photo_id, which maps to
        // photos.encrypted_blob_id rather than photos.id on the backup.
        let result = sqlx::query("DELETE FROM photos WHERE id = ? OR encrypted_blob_id = ?")
            .bind(id)
            .bind(id)
            .execute(&state.pool)
            .await;
        match result {
            Ok(r) if r.rows_affected() > 0 => {
                removed += r.rows_affected() as usize;
                // Clean up orphaned tags for the removed row.
                let _ = sqlx::query("DELETE FROM photo_tags WHERE photo_id = ?")
                    .bind(id)
                    .execute(&state.pool)
                    .await;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(photo_id = %id, "sync-deletions: failed to remove from photos: {}", e);
            }
        }
    }

    // Fallback: match by file_path for rows where the backup's
    // encrypted_blob_id differs from the primary's (each server generates
    // its own encryption blob IDs independently).
    for fp in &file_paths {
        let result = sqlx::query("DELETE FROM photos WHERE file_path = ?")
            .bind(fp)
            .execute(&state.pool)
            .await;
        match result {
            Ok(r) if r.rows_affected() > 0 => {
                removed += r.rows_affected() as usize;
                // Clean up orphaned tags — match by file_path via subquery.
                let _ = sqlx::query(
                    "DELETE FROM photo_tags WHERE photo_id IN \
                     (SELECT id FROM photos WHERE file_path = ?)",
                )
                .bind(fp)
                .execute(&state.pool)
                .await;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(file_path = %fp, "sync-deletions: failed to remove by file_path: {}", e);
            }
        }
    }

    // Also remove from the blobs table — trashed client-encrypted blobs
    // live in `blobs` rather than `photos`, so Phase 0a must clean up both.
    let mut blobs_removed = 0usize;
    for id in &ids {
        let result = sqlx::query("DELETE FROM blobs WHERE id = ?")
            .bind(id)
            .execute(&state.pool)
            .await;
        match result {
            Ok(r) if r.rows_affected() > 0 => {
                blobs_removed += r.rows_affected() as usize;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(blob_id = %id, "sync-deletions: failed to remove from blobs: {}", e);
            }
        }
    }

    if removed > 0 || blobs_removed > 0 {
        tracing::info!(
            "sync-deletions: removed {} photo(s) and {} blob(s) from gallery that are now in primary trash",
            removed,
            blobs_removed
        );
    }

    Ok(StatusCode::OK)
}
