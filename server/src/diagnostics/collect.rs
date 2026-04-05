//! Shared diagnostics collection logic.
//!
//! Both the admin-only JWT-authenticated endpoints ([`super::handlers`]) and
//! the HTTP Basic Auth external endpoints ([`super::external`]) need the same
//! metrics.  This module provides composable collector functions so each
//! section (server, database, storage, users, photos, audit, client logs,
//! backup, performance) is defined once.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use sqlx::SqlitePool;

use super::handlers::{dir_usage, disk_stats, read_cpu_seconds, read_rss_bytes, server_start};
use super::models::*;
use crate::config::AppConfig;

/// Collect server identity and resource usage.
pub async fn collect_server_info(config: &AppConfig, storage_root: &Path) -> ServerInfo {
    let (start_instant, started_at) = server_start();
    let uptime = start_instant.elapsed().as_secs();

    let (rss_bytes, cpu_secs) =
        tokio::task::spawn_blocking(|| (read_rss_bytes(), read_cpu_seconds()))
            .await
            .unwrap_or((0, 0.0));

    ServerInfo {
        version: crate::VERSION.to_string(),
        uptime_seconds: uptime,
        rust_version: env!("CARGO_PKG_RUST_VERSION", "unknown").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        memory_rss_bytes: rss_bytes,
        cpu_seconds: cpu_secs,
        pid: std::process::id(),
        storage_root: storage_root.display().to_string(),
        db_path: config.database.path.display().to_string(),
        tls_enabled: config.tls.enabled,
        max_blob_size_mb: config.storage.max_blob_size_bytes / (1024 * 1024),
        started_at: started_at.clone(),
    }
}

/// Collect SQLite database statistics (file sizes, PRAGMAs, table row counts).
pub async fn collect_database_stats(pool: &SqlitePool, db_path: &Path) -> DatabaseStats {
    let db_size = tokio::fs::metadata(db_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let wal_path = db_path.with_extension("db-wal");
    let wal_size = tokio::fs::metadata(&wal_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(pool)
        .await
        .unwrap_or_else(|_| "unknown".to_string());
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let freelist_count: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    // NOTE: These names must match the actual migration table names exactly.
    let tables = [
        "users",
        "photos",
        "blobs",
        "audit_log",
        "client_logs",
        "refresh_tokens",
        "trash_items",
        "backup_servers",
        "backup_sync_log",
        "shared_albums",
        "photo_tags",
        "encrypted_galleries",
    ];
    let mut table_counts: HashMap<String, i64> = HashMap::new();
    for table in tables {
        let sql = format!("SELECT COUNT(*) FROM {}", table);
        let count: i64 = sqlx::query_scalar(&sql).fetch_one(pool).await.unwrap_or(0);
        table_counts.insert(table.to_string(), count);
    }

    DatabaseStats {
        size_bytes: db_size,
        wal_size_bytes: wal_size,
        table_counts,
        journal_mode,
        page_size,
        page_count,
        freelist_count,
    }
}

/// Collect storage usage: directory walk + disk capacity.
pub async fn collect_storage_stats(storage_root: &Path) -> StorageStats {
    let (dir_bytes, file_count) = dir_usage(storage_root).await;
    let root = storage_root.to_path_buf();
    let (disk_total, disk_available) =
        tokio::task::spawn_blocking(move || disk_stats(&root))
            .await
            .unwrap_or((0, 0));
    let disk_used_percent = if disk_total > 0 {
        ((disk_total - disk_available) as f64 / disk_total as f64) * 100.0
    } else {
        0.0
    };

    StorageStats {
        total_bytes: dir_bytes,
        file_count,
        disk_total_bytes: disk_total,
        disk_available_bytes: disk_available,
        disk_used_percent,
    }
}

/// Collect user counts (total, admins, TOTP-enabled).
pub async fn collect_user_stats(pool: &SqlitePool) -> UserStats {
    let total_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let admin_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let totp_enabled_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE totp_enabled = 1")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    UserStats {
        total_users,
        admin_count,
        totp_enabled_count,
    }
}

/// Collect photo statistics (counts, sizes, media types, favorites, tags).
pub async fn collect_photo_stats(pool: &SqlitePool) -> PhotoStats {
    let total_photos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let encrypted_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let total_file_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM photos")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let total_thumb_bytes: i64 = 0;
    let photos_with_thumbs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE thumb_path IS NOT NULL AND thumb_path != ''",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    let favorited_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE is_favorite = 1")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let tagged_count: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT photo_id) FROM photo_tags")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let oldest_photo: Option<String> = sqlx::query_scalar("SELECT MIN(created_at) FROM photos")
        .fetch_one(pool)
        .await
        .unwrap_or(None);
    let newest_photo: Option<String> = sqlx::query_scalar("SELECT MAX(created_at) FROM photos")
        .fetch_one(pool)
        .await
        .unwrap_or(None);

    let media_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT media_type, COUNT(*) as cnt FROM photos GROUP BY media_type")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let photos_by_media_type: HashMap<String, i64> = media_rows.into_iter().collect();

    PhotoStats {
        total_photos,
        encrypted_count,
        total_file_bytes,
        total_thumb_bytes,
        photos_with_thumbs,
        photos_by_media_type,
        oldest_photo,
        newest_photo,
        favorited_count,
        tagged_count,
    }
}

/// Collect audit log summary (totals, 24h/7d counts, failures).
pub async fn collect_audit_summary(pool: &SqlitePool) -> AuditSummary {
    let now = chrono::Utc::now();
    let h24 = (now - chrono::Duration::hours(24)).to_rfc3339();
    let d7 = (now - chrono::Duration::days(7)).to_rfc3339();

    let audit_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let audit_24h: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at > ?")
        .bind(&h24)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let audit_7d: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at > ?")
        .bind(&d7)
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    let event_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT event_type, COUNT(*) as cnt FROM audit_log GROUP BY event_type")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let events_by_type: HashMap<String, i64> = event_rows.into_iter().collect();

    let failure_rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT event_type, ip_address, user_agent, created_at, details \
         FROM audit_log WHERE event_type IN ('login_failure', 'totp_login_failure', 'rate_limited', 'account_locked') \
         ORDER BY created_at DESC LIMIT 50",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let recent_failures: Vec<AuditFailureEntry> = failure_rows
        .into_iter()
        .map(
            |(event_type, ip_address, user_agent, created_at, details)| AuditFailureEntry {
                event_type,
                ip_address,
                user_agent,
                created_at,
                details,
            },
        )
        .collect();

    AuditSummary {
        total_entries: audit_total,
        entries_last_24h: audit_24h,
        entries_last_7d: audit_7d,
        events_by_type,
        recent_failures,
    }
}

/// Collect client diagnostic log summary.
pub async fn collect_client_log_summary(pool: &SqlitePool) -> ClientLogSummary {
    let now = chrono::Utc::now();
    let h24 = (now - chrono::Duration::hours(24)).to_rfc3339();
    let d7 = (now - chrono::Duration::days(7)).to_rfc3339();

    let cl_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM client_logs")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let cl_24h: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM client_logs WHERE created_at > ?")
        .bind(&h24)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let cl_7d: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM client_logs WHERE created_at > ?")
        .bind(&d7)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let cl_level_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT level, COUNT(*) as cnt FROM client_logs GROUP BY level")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let by_level: HashMap<String, i64> = cl_level_rows.into_iter().collect();
    let unique_sessions: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT session_id) FROM client_logs")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    ClientLogSummary {
        total_entries: cl_total,
        entries_last_24h: cl_24h,
        entries_last_7d: cl_7d,
        by_level,
        unique_sessions,
    }
}

/// Collect backup server and sync log summary.
pub async fn collect_backup_summary(pool: &SqlitePool) -> BackupSummary {
    let backup_servers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM backup_servers")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let total_sync_logs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM backup_sync_log")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let last_sync_at: Option<String> =
        sqlx::query_scalar("SELECT MAX(started_at) FROM backup_sync_log")
            .fetch_one(pool)
            .await
            .unwrap_or(None);

    BackupSummary {
        server_count: backup_servers,
        total_sync_logs,
        last_sync_at,
    }
}

/// Measure DB round-trip latency.
pub async fn collect_performance(pool: &SqlitePool) -> PerformanceStats {
    let t0 = Instant::now();
    let _: i64 = sqlx::query_scalar("SELECT 1")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let db_ping_ms = t0.elapsed().as_secs_f64() * 1000.0;

    PerformanceStats {
        db_ping_ms,
        cache_hit_ratio: None,
    }
}

/// Collect the complete diagnostics snapshot used by both admin and external endpoints.
pub async fn collect_full_diagnostics(
    pool: &SqlitePool,
    config: &AppConfig,
    storage_root: &Path,
) -> DiagnosticsResponse {
    let server_info = collect_server_info(config, storage_root).await;
    let database_stats = collect_database_stats(pool, &config.database.path).await;
    let storage_stats = collect_storage_stats(storage_root).await;
    let user_stats = collect_user_stats(pool).await;
    let photo_stats = collect_photo_stats(pool).await;
    let audit_summary = collect_audit_summary(pool).await;
    let client_log_summary = collect_client_log_summary(pool).await;
    let backup_summary = collect_backup_summary(pool).await;
    let performance = collect_performance(pool).await;

    DiagnosticsResponse {
        enabled: true,
        server: server_info,
        database: database_stats,
        storage: storage_stats,
        users: user_stats,
        photos: photo_stats,
        audit: audit_summary,
        client_logs: client_log_summary,
        backup: backup_summary,
        performance,
    }
}
