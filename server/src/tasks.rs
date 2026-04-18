//! Background tasks spawned at startup.
//!
//! Centralises all `tokio::spawn` calls that used to live inline in `main()`,
//! making the startup sequence easier to read and each task easier to find.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sqlx::SqlitePool;

use crate::config::AppConfig;
use crate::state::AuditBroadcast;

use crate::audit;

/// Spawn every long-running background task.
///
/// Each task runs in its own Tokio task and loops with either a fixed interval
/// or an event-driven trigger. They all tolerate transient errors gracefully —
/// a single failed cycle is logged and retried on the next tick.
pub fn spawn_all(
    pool: &SqlitePool,
    config: &AppConfig,
    storage_root_swap: &Arc<arc_swap::ArcSwap<PathBuf>>,
    scan_lock: &Arc<tokio::sync::Mutex<()>>,
    audit_tx: &tokio::sync::broadcast::Sender<AuditBroadcast>,
    storage_available: &Arc<AtomicBool>,
) {
    spawn_housekeeping(pool.clone());
    spawn_trash_purge(pool.clone(), config.storage.root.clone());
    spawn_backup_sync(pool.clone(), config.storage.root.clone(), config.backup.accept_invalid_certs);
    spawn_diagnostics_push(
        pool.clone(),
        config.storage.root.clone(),
        PathBuf::from(&config.database.path),
        config.backup.accept_invalid_certs,
    );
    spawn_log_forward(pool.clone(), audit_tx.clone(), config.backup.accept_invalid_certs);
    spawn_broadcast(pool.clone(), config.server.port);
    spawn_discovery_listener(pool.clone(), Arc::new(config.clone()));
    spawn_auto_scan(
        pool.clone(),
        storage_root_swap.clone(),
        config.scan.auto_scan_interval_secs,
        scan_lock.clone(),
        config.auth.jwt_secret.clone(),
    );
    spawn_encryption_migration(
        pool.clone(),
        config.storage.root.clone(),
        config.auth.jwt_secret.clone(),
    );
    spawn_export_cleanup(pool.clone(), storage_root_swap.clone());
    spawn_storage_health_monitor(storage_root_swap.clone(), storage_available.clone());
    spawn_dimension_repair(pool.clone(), config.storage.root.clone());
    spawn_thumbnail_orientation_repair(pool.clone(), config.storage.root.clone());
    crate::ai::processor::spawn_ai_processor(pool.clone(), config.ai.clone());
    crate::geo::processor::spawn_geo_processor(pool.clone(), config.geo.clone());
}

// ── Individual task spawners ─────────────────────────────────────────

/// Hourly housekeeping: purge expired refresh tokens, trim audit log
/// (90 days) and client diagnostic logs (14 days) in a single transaction.
fn spawn_housekeeping(pool: SqlitePool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let now = chrono::Utc::now().to_rfc3339();
            let audit_cutoff = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
            let log_cutoff = (chrono::Utc::now() - chrono::Duration::days(14)).to_rfc3339();

            match pool.begin().await {
                Ok(mut tx) => {
                    // 1. Expired / revoked refresh tokens
                    match sqlx::query(
                        "DELETE FROM refresh_tokens WHERE expires_at < ? OR (revoked = 1 AND created_at < ?)",
                    )
                    .bind(&now)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await
                    {
                        Ok(r) if r.rows_affected() > 0 => {
                            tracing::info!(
                                "Cleaned up {} expired/revoked refresh tokens",
                                r.rows_affected()
                            );
                        }
                        Err(e) => tracing::error!("Failed to clean up tokens: {}", e),
                        _ => {}
                    }

                    // 2. Audit log entries older than 90 days
                    match sqlx::query("DELETE FROM audit_log WHERE created_at < ?")
                        .bind(&audit_cutoff)
                        .execute(&mut *tx)
                        .await
                    {
                        Ok(r) if r.rows_affected() > 0 => {
                            tracing::info!(
                                "Cleaned up {} old audit log entries (> 90 days)",
                                r.rows_affected()
                            );
                        }
                        Err(e) => tracing::error!("Failed to clean up audit log: {}", e),
                        _ => {}
                    }

                    // 3. Client diagnostic logs older than 14 days
                    match sqlx::query("DELETE FROM client_logs WHERE created_at < ?")
                        .bind(&log_cutoff)
                        .execute(&mut *tx)
                        .await
                    {
                        Ok(r) if r.rows_affected() > 0 => {
                            tracing::info!(
                                "Cleaned up {} old client log entries (> 14 days)",
                                r.rows_affected()
                            );
                        }
                        Err(e) => tracing::error!("Failed to clean up client logs: {}", e),
                        _ => {}
                    }

                    if let Err(e) = tx.commit().await {
                        tracing::error!("Housekeeping transaction commit failed: {}", e);
                    } else {
                        audit::log_background(
                            &pool,
                            audit::AuditEvent::HousekeepingComplete,
                            Some(serde_json::json!({"task": "housekeeping"})),
                        );
                    }
                }
                Err(e) => tracing::error!("Housekeeping: failed to begin transaction: {}", e),
            }
        }
    });
}

/// Hourly purge of trash items past their retention window.
fn spawn_trash_purge(pool: SqlitePool, storage_root: PathBuf) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            crate::trash::purge::purge_expired_trash(&pool, &storage_root).await;
        }
    });
}

/// Periodic sync to registered backup servers.
fn spawn_backup_sync(pool: SqlitePool, storage_root: PathBuf, accept_invalid_certs: bool) {
    tokio::spawn(async move {
        crate::backup::sync::background_sync_task(pool, storage_root, accept_invalid_certs).await;
    });
}

/// Push diagnostics snapshot from backup to primary every 15 min.
fn spawn_diagnostics_push(pool: SqlitePool, storage_root: PathBuf, db_path: PathBuf, accept_invalid_certs: bool) {
    tokio::spawn(async move {
        crate::backup::diagnostics::background_diagnostics_push_task(pool, storage_root, db_path, accept_invalid_certs)
            .await;
    });
}

/// Forward audit logs from backup to primary in real time.
fn spawn_log_forward(pool: SqlitePool, audit_tx: tokio::sync::broadcast::Sender<AuditBroadcast>, accept_invalid_certs: bool) {
    tokio::spawn(async move {
        crate::backup::diagnostics::background_log_forward_task(pool, audit_tx, accept_invalid_certs).await;
    });
}

/// UDP LAN broadcast so other servers can discover us.
fn spawn_broadcast(pool: SqlitePool, server_port: u16) {
    tokio::spawn(async move {
        crate::backup::broadcast::background_broadcast_task(pool, server_port).await;
    });
}

/// Dedicated discovery listener on the configured discovery port.
fn spawn_discovery_listener(pool: SqlitePool, config: Arc<AppConfig>) {
    tokio::spawn(async move {
        crate::backup::discovery::run_discovery_listener(pool, config).await;
    });
}

/// Periodic filesystem scan that registers new files.
fn spawn_auto_scan(
    pool: SqlitePool,
    storage_root_swap: Arc<arc_swap::ArcSwap<PathBuf>>,
    interval_secs: u64,
    scan_lock: Arc<tokio::sync::Mutex<()>>,
    jwt_secret: String,
) {
    tokio::spawn(async move {
        crate::backup::autoscan::background_auto_scan_task(
            pool,
            storage_root_swap,
            interval_secs,
            scan_lock,
            jwt_secret,
        )
        .await;
    });
}

/// Resume any interrupted encryption migration on startup.
fn spawn_encryption_migration(pool: SqlitePool, storage_root: PathBuf, jwt_secret: String) {
    tokio::spawn(async move {
        crate::photos::server_migrate::resume_migration_on_startup(pool, storage_root, jwt_secret)
            .await;
    });
}

/// Hourly cleanup of expired export zip files (24-hour TTL).
fn spawn_export_cleanup(pool: SqlitePool, storage_root_swap: Arc<arc_swap::ArcSwap<PathBuf>>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let now = chrono::Utc::now().to_rfc3339();
            let storage_root = storage_root_swap.load();

            // Find expired export files
            let expired: Vec<(String, String, String)> = match sqlx::query_as::<_, (String, String, String)>(
                "SELECT ef.id, ef.file_path, ef.job_id FROM export_files ef WHERE ef.expires_at < ?",
            )
            .bind(&now)
            .fetch_all(&pool)
            .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::error!("Export cleanup: failed to query expired files: {}", e);
                    continue;
                }
            };

            if expired.is_empty() {
                continue;
            }

            let mut deleted_count = 0u64;
            let mut job_ids = std::collections::HashSet::new();

            for (file_id, file_path, job_id) in &expired {
                // Delete file from disk
                let full_path = storage_root.join(file_path);
                if tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
                    if let Err(e) = tokio::fs::remove_file(&full_path).await {
                        tracing::warn!(file_id = %file_id, error = %e, "Failed to delete expired export file");
                    }
                }

                // Delete DB record
                if let Err(e) = sqlx::query("DELETE FROM export_files WHERE id = ?")
                    .bind(file_id)
                    .execute(&pool)
                    .await
                {
                    tracing::error!(file_id = %file_id, error = %e, "Failed to delete export file record");
                } else {
                    deleted_count += 1;
                }

                job_ids.insert(job_id.clone());
            }

            // Clean up empty export directories and completed/failed jobs with no remaining files
            for job_id in &job_ids {
                let remaining: Option<(i64,)> = sqlx::query_as(
                    "SELECT COUNT(*) FROM export_files WHERE job_id = ?",
                )
                .bind(job_id)
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();

                if remaining.map(|(c,)| c).unwrap_or(0) == 0 {
                    // Remove export directory
                    let export_dir = storage_root.join("exports").join(job_id);
                    if tokio::fs::try_exists(&export_dir).await.unwrap_or(false) {
                        let _ = tokio::fs::remove_dir_all(&export_dir).await;
                    }
                    // Delete the job record
                    let _ = sqlx::query("DELETE FROM export_jobs WHERE id = ?")
                        .bind(job_id)
                        .execute(&pool)
                        .await;
                }
            }

            if deleted_count > 0 {
                tracing::info!("Export cleanup: removed {} expired export files", deleted_count);
                audit::log_background(
                    &pool,
                    audit::AuditEvent::HousekeepingComplete,
                    Some(serde_json::json!({"task": "export_cleanup", "files_removed": deleted_count})),
                );
            }
        }
    });
}

/// Storage health monitor — probes the storage root every 10 seconds by
/// writing and reading back a small sentinel file.  If the probe fails
/// (e.g. network drive disconnected), the `storage_available` flag is set
/// to `false` and handlers return 503 instead of hanging on stale I/O.
///
/// When the storage comes back, the flag is restored and normal operation
/// resumes automatically.
fn spawn_storage_health_monitor(
    storage_root_swap: Arc<arc_swap::ArcSwap<PathBuf>>,
    storage_available: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        let mut was_available = true;

        loop {
            interval.tick().await;

            let storage_root = (**storage_root_swap.load()).clone();
            let probe_path = storage_root.join(".storage_probe");

            let probe_ok = probe_storage(&probe_path).await;

            let previously_available = storage_available.load(Ordering::Relaxed);
            storage_available.store(probe_ok, Ordering::Relaxed);

            if probe_ok && !previously_available {
                tracing::info!(
                    "Storage reconnected — resuming normal operation (root: {:?})",
                    storage_root
                );
                was_available = true;
            } else if !probe_ok && was_available {
                tracing::error!(
                    "Storage unavailable — probe failed at {:?}. \
                     Will retry every 10 seconds until reconnected.",
                    storage_root
                );
                was_available = false;
            } else if !probe_ok {
                tracing::warn!(
                    "Storage still unavailable — retrying in 10s (root: {:?})",
                    storage_root
                );
            }
        }
    });
}

/// Attempt to write and read back a sentinel file to verify storage is healthy.
async fn probe_storage(probe_path: &std::path::Path) -> bool {
    use tokio::io::AsyncWriteExt;

    let payload = b"storage-health-probe";

    // Try to write the probe file
    let write_result = async {
        let mut file = tokio::fs::File::create(probe_path).await?;
        file.write_all(payload).await?;
        file.flush().await?;
        Ok::<(), std::io::Error>(())
    }
    .await;

    if write_result.is_err() {
        // Clean up on failure (best-effort)
        let _ = tokio::fs::remove_file(probe_path).await;
        return false;
    }

    // Try to read it back and verify contents
    match tokio::fs::read(probe_path).await {
        Ok(data) if data == payload => {
            // Clean up probe file (best-effort)
            let _ = tokio::fs::remove_file(probe_path).await;
            true
        }
        _ => {
            let _ = tokio::fs::remove_file(probe_path).await;
            false
        }
    }
}

/// One-time startup task: re-read EXIF orientation for all photos and fix
/// width/height where orientations 5-8 had the dimensions un-swapped.
fn spawn_dimension_repair(pool: SqlitePool, storage_root: PathBuf) {
    tokio::spawn(async move {
        crate::photos::metadata::repair_orientation_dimensions(&pool, &storage_root).await;
    });
}

/// One-time startup task: regenerate thumbnails for photos with EXIF
/// orientation ≥ 2 so portrait camera photos display correctly.
fn spawn_thumbnail_orientation_repair(pool: SqlitePool, storage_root: PathBuf) {
    tokio::spawn(async move {
        crate::photos::thumbnail::repair_thumbnail_orientation(&pool, &storage_root).await;
    });
}
