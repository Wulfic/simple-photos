//! Cleanup endpoint: remove original plain files for photos that have been
//! successfully encrypted. Only available in encrypted mode when there are
//! plain-file originals still on disk.

use axum::extract::State;
use axum::Json;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// Counts how many photos still have original plain files on disk alongside
/// their encrypted blobs. Used by the frontend to decide whether to show the
/// "Clean up backed-up photos" option.
///
/// GET /photos/cleanup-status
pub async fn cleanup_status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Only meaningful in encrypted mode
    let mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    if mode != "encrypted" {
        return Ok(Json(serde_json::json!({
            "cleanable_count": 0,
            "cleanable_bytes": 0i64,
        })));
    }

    // Photos that are encrypted AND still have a non-empty file_path
    let row: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) \
         FROM photos \
         WHERE user_id = ? AND encrypted_blob_id IS NOT NULL \
           AND file_path IS NOT NULL AND file_path != ''",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({
        "cleanable_count": row.0,
        "cleanable_bytes": row.1,
    })))
}

/// Delete original plain-mode files (photo/video + thumbnail + web preview)
/// for every photo that has been successfully encrypted. Clears `file_path`
/// and `thumb_path` in the DB so they won't be served or re-cleaned.
///
/// POST /photos/cleanup
pub async fn cleanup_plain_files(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Only allowed in encrypted mode
    let mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    if mode != "encrypted" {
        return Err(AppError::BadRequest(
            "Cleanup is only available in encrypted mode".into(),
        ));
    }

    // Ensure no migration is in progress
    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "idle".to_string());

    if mig_status != "idle" {
        return Err(AppError::BadRequest(
            "Cannot clean up while a migration is in progress".into(),
        ));
    }

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();

    let (cleaned, errors) = cleanup_plain_files_internal(&state.pool, &auth.user_id, &storage_root).await?;

    Ok(Json(serde_json::json!({
        "cleaned": cleaned,
        "errors": errors,
        "message": format!(
            "Cleaned up {} file{}{}.",
            cleaned,
            if cleaned != 1 { "s" } else { "" },
            if errors > 0 { format!(", {} error(s)", errors) } else { String::new() },
        ),
    })))
}

/// Internal function to execute cleanup logic (also used automatically after migration).
pub async fn cleanup_plain_files_internal(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    storage_root: &std::path::Path,
) -> Result<(u32, u32), AppError> {
    // Fetch all encrypted photos that still have plain originals
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, file_path, thumb_path \
         FROM photos \
         WHERE user_id = ? AND encrypted_blob_id IS NOT NULL \
           AND file_path IS NOT NULL AND file_path != ''",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok((0, 0));
    }

    let mut cleaned = 0u32;
    let mut errors = 0u32;

    for (photo_id, file_path, thumb_path) in &rows {
        // Delete original media file
        let abs_path = storage_root.join(file_path);
        if let Err(e) = tokio::fs::remove_file(&abs_path).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    photo_id = %photo_id,
                    path = %abs_path.display(),
                    error = %e,
                    "Failed to delete original file during cleanup"
                );
                errors += 1;
                continue;
            }
        }

        // Delete thumbnail
        if let Some(tp) = thumb_path {
            let thumb_abs = storage_root.join(tp);
            let _ = tokio::fs::remove_file(&thumb_abs).await;
        }

        // Delete web preview (if one exists)
        let preview_dir = storage_root.join(".web_previews");
        // Web previews use the pattern {id}.web.{ext}
        if let Ok(mut entries) = tokio::fs::read_dir(&preview_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{}.", photo_id))
                {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
            }
        }

        // Clear file_path and thumb_path in DB so this photo is no longer
        // served via plain-mode endpoints
        sqlx::query(
            "UPDATE photos SET file_path = '', thumb_path = NULL \
             WHERE id = ? AND user_id = ?",
        )
        .bind(photo_id)
        .bind(user_id)
        .execute(pool)
        .await?;

        cleaned += 1;
    }

    // Try to remove empty parent directories left behind
    cleanup_empty_dirs(storage_root).await;

    tracing::info!(
        user_id = %user_id,
        cleaned = cleaned,
        errors = errors,
        "Plain file cleanup completed"
    );

    Ok((cleaned, errors))
}

/// Walk the storage root and remove any empty directories (except the root
/// itself and known server-managed directories).
async fn cleanup_empty_dirs(root: &std::path::Path) {
    // Collect directories bottom-up so children are removed before parents
    let mut dirs = Vec::new();
    collect_dirs(root, &mut dirs).await;

    // Sort by depth descending (deepest first)
    dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()));

    for dir in dirs {
        if dir == root {
            continue;
        }
        // Only remove if truly empty
        if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
            if entries.next_entry().await.ok().flatten().is_none() {
                let _ = tokio::fs::remove_dir(&dir).await;
            }
        }
    }
}

async fn collect_dirs(path: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(mut entries) = tokio::fs::read_dir(path).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    let p = entry.path();
                    out.push(p.clone());
                    Box::pin(collect_dirs(&p, out)).await;
                }
            }
        }
    }
}
