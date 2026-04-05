//! Background task: periodic purge of expired trash items.
//!
//! Called hourly to permanently delete items whose `expires_at` has passed.
//! File deletion only happens after the DB transaction commits, and files
//! are only removed if no other photo row still references them.

use chrono::Utc;

/// Purge expired trash items.
/// Called periodically (e.g. every hour) to permanently delete items
/// whose `expires_at` has passed.
pub async fn purge_expired_trash(pool: &sqlx::SqlitePool, storage_root: &std::path::Path) {
    let now = Utc::now().to_rfc3339();

    // Begin transaction — ref-count checks + batch DELETE must be atomic
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for trash purge: {}", e);
            return;
        }
    };

    // Fetch expired items for file cleanup
    let expired: Vec<(String, String, Option<String>)> = match sqlx::query_as(
        "SELECT id, file_path, thumb_path FROM trash_items WHERE expires_at <= ?",
    )
    .bind(&now)
    .fetch_all(&mut *tx)
    .await
    {
        Ok(items) => items,
        Err(e) => {
            tracing::error!("Failed to query expired trash items: {}", e);
            return;
        }
    };

    if expired.is_empty() {
        return;
    }

    // Build list of files safe to delete and IDs to remove, all within the transaction
    let mut files_to_delete: Vec<std::path::PathBuf> = Vec::new();
    let mut ids_to_delete: Vec<&str> = Vec::new();

    for (id, file_path, thumb_path) in &expired {
        // Only delete files if no other photo row still references them.
        // On DB error, skip this item entirely — do NOT default to 0
        // because that would cause irreversible file deletion.
        let other_refs: i64 =
            match sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE file_path = ?")
                .bind(file_path)
                .fetch_one(&mut *tx)
                .await
            {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!("DB error checking file refs for {}: {} — skipping", id, e);
                    continue;
                }
            };

        if other_refs == 0 {
            files_to_delete.push(storage_root.join(file_path));
        }

        if let Some(tp) = thumb_path {
            let other_thumb_refs: i64 =
                match sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE thumb_path = ?")
                    .bind(tp)
                    .fetch_one(&mut *tx)
                    .await
                {
                    Ok(n) => n,
                    Err(e) => {
                        tracing::error!(
                            "DB error checking thumb refs for {}: {} — skipping",
                            id,
                            e
                        );
                        continue;
                    }
                };

            if other_thumb_refs == 0 {
                files_to_delete.push(storage_root.join(tp));
            }
        }

        ids_to_delete.push(id.as_str());
    }

    // Delete all expired rows within the transaction
    for id in &ids_to_delete {
        if let Err(e) = sqlx::query("DELETE FROM trash_items WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
        {
            tracing::error!("Failed to delete expired trash item {}: {}", id, e);
        }
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit trash purge transaction: {}", e);
        return;
    }

    // Delete files from disk AFTER commit
    for path in &files_to_delete {
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }

    let purged = ids_to_delete.len();
    if purged > 0 {
        tracing::info!("Purged {} expired trash items", purged);
        crate::audit::log_background(
            pool,
            crate::audit::AuditEvent::TrashPurgeComplete,
            Some(serde_json::json!({"purged_count": purged})),
        );
    }
}
