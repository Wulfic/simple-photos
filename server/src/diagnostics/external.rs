//! External diagnostics API — authenticated via HTTP Basic Auth.
//!
//! These endpoints let other servers (monitoring systems, dashboards, etc.)
//! pull diagnostics data using a valid admin username and password.
//!
//! Authentication: standard HTTP Basic Auth header
//!   `Authorization: Basic base64(username:password)`
//!
//! All endpoints require admin role.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use std::collections::HashMap;
use std::time::Instant;

use crate::error::AppError;
use crate::state::AppState;

use super::handlers::{server_start, read_rss_bytes, read_cpu_seconds, disk_stats, dir_usage};
use super::models::*;

// ── Basic Auth Helper ─────────────────────────────────────────────────────

/// Validate HTTP Basic Auth credentials against the users table.
/// Returns the user ID if valid and the user has admin role.
async fn require_basic_auth_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, AppError> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::Unauthorized("Missing Authorization header. Use Basic Auth.".into())
        })?;

    // Parse "Basic base64(username:password)"
    let encoded = auth_header
        .strip_prefix("Basic ")
        .or_else(|| auth_header.strip_prefix("basic "))
        .ok_or_else(|| {
            AppError::Unauthorized("Invalid Authorization header. Expected Basic Auth.".into())
        })?;

    let decoded = base64_decode(encoded).map_err(|_| {
        AppError::Unauthorized("Invalid Base64 in Authorization header".into())
    })?;

    let (username, password) = decoded
        .split_once(':')
        .ok_or_else(|| AppError::Unauthorized("Invalid Basic Auth format".into()))?;

    if username.is_empty() || password.is_empty() {
        return Err(AppError::Unauthorized(
            "Username and password are required".into(),
        ));
    }

    // Look up user
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, password_hash, COALESCE(role, 'user') FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(&state.pool)
    .await?;

    let (user_id, password_hash, role) = match row {
        Some(r) => r,
        None => {
            // Constant-time: still run bcrypt to prevent timing attacks
            let _ = bcrypt::verify(
                password,
                "$2b$12$LJ3m9blCPMEtJDZk4CYOqe4CIH55aN38bwSqggfgA1mJm/kzbyPhK",
            );
            return Err(AppError::Unauthorized(
                "Invalid username or password".into(),
            ));
        }
    };

    let valid = bcrypt::verify(password, &password_hash)
        .map_err(|e| AppError::Internal(format!("bcrypt error: {}", e)))?;

    if !valid {
        return Err(AppError::Unauthorized(
            "Invalid username or password".into(),
        ));
    }

    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    Ok(user_id)
}

/// Simple base64 decoder (standard alphabet, no padding required).
fn base64_decode(input: &str) -> Result<String, ()> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input.trim())
        .map_err(|_| ())?;
    String::from_utf8(bytes).map_err(|_| ())
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/external/diagnostics/health — lightweight health for monitoring
// ═══════════════════════════════════════════════════════════════════════════

pub async fn external_health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ExternalHealthResponse>, AppError> {
    require_basic_auth_admin(&state, &headers).await?;

    let (start_instant, started_at) = server_start();
    let uptime = start_instant.elapsed().as_secs();

    let pool = &state.pool;

    // DB ping
    let t0 = Instant::now();
    let _: i64 = sqlx::query_scalar("SELECT 1")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let db_ping_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // Disk usage
    let storage_root = state.storage_root.read().await.clone();
    let (disk_total, disk_available) = disk_stats(&storage_root);
    let disk_used_percent = if disk_total > 0 {
        ((disk_total - disk_available) as f64 / disk_total as f64) * 100.0
    } else {
        0.0
    };

    // Counts
    let total_photos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let total_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    Ok(Json(ExternalHealthResponse {
        status: "ok".into(),
        version: "0.6.9".into(),
        uptime_seconds: uptime,
        started_at: started_at.clone(),
        memory_rss_bytes: read_rss_bytes(),
        cpu_seconds: read_cpu_seconds(),
        db_ping_ms,
        disk_used_percent,
        total_photos,
        total_users,
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/external/diagnostics — full metrics (same as admin endpoint)
// ═══════════════════════════════════════════════════════════════════════════

pub async fn external_full(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;

    require_basic_auth_admin(&state, &headers).await?;

    let (start_instant, started_at) = server_start();
    let uptime = start_instant.elapsed().as_secs();
    let pool = &state.pool;

    // ── Server info ───────────────────────────────────────────────────
    let storage_root = state.storage_root.read().await.clone();
    let server_info = ServerInfo {
        version: "0.6.9".to_string(),
        uptime_seconds: uptime,
        rust_version: env!("CARGO_PKG_RUST_VERSION", "unknown").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        memory_rss_bytes: read_rss_bytes(),
        cpu_seconds: read_cpu_seconds(),
        pid: std::process::id(),
        storage_root: storage_root.display().to_string(),
        db_path: state.config.database.path.display().to_string(),
        tls_enabled: state.config.tls.enabled,
        max_blob_size_mb: state.config.storage.max_blob_size_bytes / (1024 * 1024),
        started_at: started_at.clone(),
    };

    // ── Database stats ────────────────────────────────────────────────
    let db_path = &state.config.database.path;
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

    let tables = [
        "users", "photos", "blobs", "audit_log", "client_logs",
        "refresh_tokens", "trash", "backup_servers", "sync_logs",
        "shared_albums", "photo_tags", "secure_galleries",
    ];
    let mut table_counts: HashMap<String, i64> = HashMap::new();
    for table in tables {
        let sql = format!("SELECT COUNT(*) FROM {}", table);
        let count: i64 = sqlx::query_scalar(&sql)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
        table_counts.insert(table.to_string(), count);
    }

    let database_stats = DatabaseStats {
        size_bytes: db_size,
        wal_size_bytes: wal_size,
        table_counts,
        journal_mode,
        page_size,
        page_count,
        freelist_count,
    };

    // ── Storage stats ─────────────────────────────────────────────────
    let (dir_bytes, file_count) = dir_usage(&storage_root).await;
    let (disk_total, disk_available) = disk_stats(&storage_root);
    let disk_used_percent = if disk_total > 0 {
        ((disk_total - disk_available) as f64 / disk_total as f64) * 100.0
    } else {
        0.0
    };

    let storage_stats = StorageStats {
        total_bytes: dir_bytes,
        file_count,
        disk_total_bytes: disk_total,
        disk_available_bytes: disk_available,
        disk_used_percent,
    };

    // ── User stats ────────────────────────────────────────────────────
    let total_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let admin_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let totp_enabled_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE totp_enabled = 1")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    let user_stats = UserStats { total_users, admin_count, totp_enabled_count };

    // ── Photo stats ───────────────────────────────────────────────────
    let total_photos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
        .fetch_one(pool).await.unwrap_or(0);
    let encrypted_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NOT NULL")
            .fetch_one(pool).await.unwrap_or(0);
    let plain_count = total_photos - encrypted_count;
    let total_file_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM photos")
            .fetch_one(pool).await.unwrap_or(0);
    // No dedicated thumb_size column — not tracked in the schema
    let total_thumb_bytes: i64 = 0;
    let photos_with_thumbs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE thumb_path IS NOT NULL AND thumb_path != ''")
            .fetch_one(pool).await.unwrap_or(0);
    let favorited_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE is_favorite = 1")
            .fetch_one(pool).await.unwrap_or(0);
    let tagged_count: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT photo_id) FROM photo_tags")
            .fetch_one(pool).await.unwrap_or(0);
    let oldest_photo: Option<String> =
        sqlx::query_scalar("SELECT MIN(created_at) FROM photos")
            .fetch_one(pool).await.unwrap_or(None);
    let newest_photo: Option<String> =
        sqlx::query_scalar("SELECT MAX(created_at) FROM photos")
            .fetch_one(pool).await.unwrap_or(None);

    let media_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT media_type, COUNT(*) as cnt FROM photos GROUP BY media_type",
    )
    .fetch_all(pool).await.unwrap_or_default();
    let photos_by_media_type: HashMap<String, i64> = media_rows.into_iter().collect();

    let photo_stats = PhotoStats {
        total_photos, encrypted_count, plain_count, total_file_bytes,
        total_thumb_bytes, photos_with_thumbs, photos_by_media_type,
        oldest_photo, newest_photo, favorited_count, tagged_count,
    };

    // ── Audit summary ─────────────────────────────────────────────────
    let now = chrono::Utc::now();
    let h24 = (now - chrono::Duration::hours(24)).to_rfc3339();
    let d7 = (now - chrono::Duration::days(7)).to_rfc3339();

    let audit_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(pool).await.unwrap_or(0);
    let audit_24h: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at > ?")
            .bind(&h24).fetch_one(pool).await.unwrap_or(0);
    let audit_7d: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at > ?")
            .bind(&d7).fetch_one(pool).await.unwrap_or(0);

    let event_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT event_type, COUNT(*) as cnt FROM audit_log GROUP BY event_type",
    )
    .fetch_all(pool).await.unwrap_or_default();
    let events_by_type: HashMap<String, i64> = event_rows.into_iter().collect();

    let failure_rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT event_type, ip_address, user_agent, created_at, details \
         FROM audit_log WHERE event_type IN ('login_failure', 'totp_login_failure', 'rate_limited', 'account_locked') \
         ORDER BY created_at DESC LIMIT 50",
    )
    .fetch_all(pool).await.unwrap_or_default();

    let recent_failures: Vec<AuditFailureEntry> = failure_rows
        .into_iter()
        .map(|(event_type, ip_address, user_agent, created_at, details)| {
            AuditFailureEntry { event_type, ip_address, user_agent, created_at, details }
        })
        .collect();

    let audit_summary = AuditSummary {
        total_entries: audit_total, entries_last_24h: audit_24h,
        entries_last_7d: audit_7d, events_by_type, recent_failures,
    };

    // ── Client log summary ────────────────────────────────────────────
    let cl_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM client_logs")
        .fetch_one(pool).await.unwrap_or(0);
    let cl_24h: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM client_logs WHERE created_at > ?")
            .bind(&h24).fetch_one(pool).await.unwrap_or(0);
    let cl_7d: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM client_logs WHERE created_at > ?")
            .bind(&d7).fetch_one(pool).await.unwrap_or(0);
    let cl_level_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT level, COUNT(*) as cnt FROM client_logs GROUP BY level",
    )
    .fetch_all(pool).await.unwrap_or_default();
    let by_level: HashMap<String, i64> = cl_level_rows.into_iter().collect();
    let unique_sessions: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT session_id) FROM client_logs")
            .fetch_one(pool).await.unwrap_or(0);

    let client_log_summary = ClientLogSummary {
        total_entries: cl_total, entries_last_24h: cl_24h,
        entries_last_7d: cl_7d, by_level, unique_sessions,
    };

    // ── Backup summary ────────────────────────────────────────────────
    let backup_servers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM backup_servers")
        .fetch_one(pool).await.unwrap_or(0);
    let total_sync_logs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_logs")
        .fetch_one(pool).await.unwrap_or(0);
    let last_sync_at: Option<String> = sqlx::query_scalar("SELECT MAX(created_at) FROM sync_logs")
        .fetch_one(pool).await.unwrap_or(None);

    let backup_summary = BackupSummary {
        server_count: backup_servers, total_sync_logs, last_sync_at,
    };

    // ── Performance ───────────────────────────────────────────────────
    let t0 = Instant::now();
    let _: i64 = sqlx::query_scalar("SELECT 1")
        .fetch_one(pool).await.unwrap_or(0);
    let db_ping_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let performance = PerformanceStats { db_ping_ms, cache_hit_ratio: None };

    Ok(Json(DiagnosticsResponse {
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
    }).into_response())
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/external/diagnostics/storage — storage-focused metrics
// ═══════════════════════════════════════════════════════════════════════════

pub async fn external_storage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ExternalStorageResponse>, AppError> {
    require_basic_auth_admin(&state, &headers).await?;

    let pool = &state.pool;
    let storage_root = state.storage_root.read().await.clone();

    // Storage
    let (dir_bytes, file_count) = dir_usage(&storage_root).await;
    let (disk_total, disk_available) = disk_stats(&storage_root);
    let disk_used_percent = if disk_total > 0 {
        ((disk_total - disk_available) as f64 / disk_total as f64) * 100.0
    } else {
        0.0
    };

    let storage = StorageStats {
        total_bytes: dir_bytes,
        file_count,
        disk_total_bytes: disk_total,
        disk_available_bytes: disk_available,
        disk_used_percent,
    };

    // Photos
    let total_photos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
        .fetch_one(pool).await.unwrap_or(0);
    let total_file_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM photos")
            .fetch_one(pool).await.unwrap_or(0);
    // No dedicated thumb_size column — not tracked in the schema
    let total_thumb_bytes: i64 = 0;
    let media_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT media_type, COUNT(*) as cnt FROM photos GROUP BY media_type",
    )
    .fetch_all(pool).await.unwrap_or_default();
    let photos_by_media_type: HashMap<String, i64> = media_rows.into_iter().collect();

    let photos = ExternalPhotoStorageStats {
        total_photos,
        total_file_bytes,
        total_thumb_bytes,
        photos_by_media_type,
    };

    // Database size
    let db_path = &state.config.database.path;
    let db_size = tokio::fs::metadata(db_path).await.map(|m| m.len()).unwrap_or(0);
    let wal_path = db_path.with_extension("db-wal");
    let wal_size = tokio::fs::metadata(&wal_path).await.map(|m| m.len()).unwrap_or(0);

    let database = ExternalDatabaseSize {
        size_bytes: db_size,
        wal_size_bytes: wal_size,
    };

    Ok(Json(ExternalStorageResponse { storage, photos, database }))
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/external/diagnostics/audit — audit & security summary
// ═══════════════════════════════════════════════════════════════════════════

pub async fn external_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ExternalAuditResponse>, AppError> {
    require_basic_auth_admin(&state, &headers).await?;

    let pool = &state.pool;
    let now = chrono::Utc::now();
    let h24 = (now - chrono::Duration::hours(24)).to_rfc3339();
    let d7 = (now - chrono::Duration::days(7)).to_rfc3339();

    // Audit
    let audit_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(pool).await.unwrap_or(0);
    let audit_24h: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at > ?")
            .bind(&h24).fetch_one(pool).await.unwrap_or(0);
    let audit_7d: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at > ?")
            .bind(&d7).fetch_one(pool).await.unwrap_or(0);

    let event_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT event_type, COUNT(*) as cnt FROM audit_log GROUP BY event_type",
    )
    .fetch_all(pool).await.unwrap_or_default();
    let events_by_type: HashMap<String, i64> = event_rows.into_iter().collect();

    let failure_rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT event_type, ip_address, user_agent, created_at, details \
         FROM audit_log WHERE event_type IN ('login_failure', 'totp_login_failure', 'rate_limited', 'account_locked') \
         ORDER BY created_at DESC LIMIT 50",
    )
    .fetch_all(pool).await.unwrap_or_default();

    let recent_failures: Vec<AuditFailureEntry> = failure_rows.into_iter()
        .map(|(event_type, ip_address, user_agent, created_at, details)| {
            AuditFailureEntry { event_type, ip_address, user_agent, created_at, details }
        })
        .collect();

    let audit = AuditSummary {
        total_entries: audit_total, entries_last_24h: audit_24h,
        entries_last_7d: audit_7d, events_by_type, recent_failures,
    };

    // Users
    let total_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool).await.unwrap_or(0);
    let admin_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
            .fetch_one(pool).await.unwrap_or(0);
    let totp_enabled_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE totp_enabled = 1")
            .fetch_one(pool).await.unwrap_or(0);

    let users = UserStats { total_users, admin_count, totp_enabled_count };

    // Client logs
    let cl_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM client_logs")
        .fetch_one(pool).await.unwrap_or(0);
    let cl_24h: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM client_logs WHERE created_at > ?")
            .bind(&h24).fetch_one(pool).await.unwrap_or(0);
    let cl_7d: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM client_logs WHERE created_at > ?")
            .bind(&d7).fetch_one(pool).await.unwrap_or(0);
    let cl_level_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT level, COUNT(*) as cnt FROM client_logs GROUP BY level",
    )
    .fetch_all(pool).await.unwrap_or_default();
    let by_level: HashMap<String, i64> = cl_level_rows.into_iter().collect();
    let unique_sessions: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT session_id) FROM client_logs")
            .fetch_one(pool).await.unwrap_or(0);

    let client_logs = ClientLogSummary {
        total_entries: cl_total, entries_last_24h: cl_24h,
        entries_last_7d: cl_7d, by_level, unique_sessions,
    };

    Ok(Json(ExternalAuditResponse { audit, users, client_logs }))
}
