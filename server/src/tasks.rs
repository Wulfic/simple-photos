//! Background tasks spawned at startup.
//!
//! Centralises all `tokio::spawn` calls that used to live inline in `main()`,
//! making the startup sequence easier to read and each task easier to find.

use std::path::PathBuf;
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
) {
    spawn_housekeeping(pool.clone());
    spawn_trash_purge(pool.clone(), config.storage.root.clone());
    spawn_backup_sync(pool.clone(), config.storage.root.clone());
    spawn_diagnostics_push(
        pool.clone(),
        config.storage.root.clone(),
        PathBuf::from(&config.database.path),
    );
    spawn_log_forward(pool.clone(), audit_tx.clone());
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
fn spawn_backup_sync(pool: SqlitePool, storage_root: PathBuf) {
    tokio::spawn(async move {
        crate::backup::sync::background_sync_task(pool, storage_root).await;
    });
}

/// Push diagnostics snapshot from backup to primary every 15 min.
fn spawn_diagnostics_push(pool: SqlitePool, storage_root: PathBuf, db_path: PathBuf) {
    tokio::spawn(async move {
        crate::backup::diagnostics::background_diagnostics_push_task(pool, storage_root, db_path)
            .await;
    });
}

/// Forward audit logs from backup to primary in real time.
fn spawn_log_forward(pool: SqlitePool, audit_tx: tokio::sync::broadcast::Sender<AuditBroadcast>) {
    tokio::spawn(async move {
        crate::backup::diagnostics::background_log_forward_task(pool, audit_tx).await;
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
