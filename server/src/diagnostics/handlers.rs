use axum::extract::{Query, State};
use axum::Json;
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::time::Instant;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::*;

/// Lazily initialised server start time – set once on first diagnostics call.
static SERVER_START: std::sync::OnceLock<(Instant, String)> = std::sync::OnceLock::new();

pub(crate) fn server_start() -> &'static (Instant, String) {
    SERVER_START.get_or_init(|| (Instant::now(), chrono::Utc::now().to_rfc3339()))
}

// ── Helpers ───────────────────────────────────────────────────────────────

async fn require_admin(auth: &AuthUser, state: &AppState) -> Result<(), AppError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await?;
    if role.as_deref() != Some("admin") {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    Ok(())
}

/// Read `/proc/self/status` on Linux to get VmRSS in bytes.
#[cfg(target_os = "linux")]
pub(crate) fn read_rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| {
                    l.split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<u64>().ok())
                })
                .map(|kb| kb * 1024) // VmRSS is in kB
        })
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_rss_bytes() -> u64 {
    0
}

/// Read `/proc/self/stat` on Linux to get user+system CPU time in seconds.
#[cfg(target_os = "linux")]
pub(crate) fn read_cpu_seconds() -> f64 {
    let ticks_per_sec = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|s| {
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() > 14 {
                let utime: f64 = parts[13].parse().unwrap_or(0.0);
                let stime: f64 = parts[14].parse().unwrap_or(0.0);
                Some((utime + stime) / ticks_per_sec)
            } else {
                None
            }
        })
        .unwrap_or(0.0)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_cpu_seconds() -> f64 {
    0.0
}

/// Get disk usage via statvfs on Unix.
#[cfg(unix)]
pub(crate) fn disk_stats(path: &std::path::Path) -> (u64, u64) {
    use std::ffi::CString;
    let c_path = match CString::new(path.to_str().unwrap_or("/")) {
        Ok(p) => p,
        Err(_) => return (0, 0),
    };
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            let total = stat.f_blocks as u64 * stat.f_frsize as u64;
            let available = stat.f_bavail as u64 * stat.f_frsize as u64;
            (total, available)
        } else {
            (0, 0)
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn disk_stats(_path: &std::path::Path) -> (u64, u64) {
    (0, 0)
}

/// Walk a directory tree and sum file sizes + count.
pub(crate) async fn dir_usage(root: &std::path::Path) -> (u64, u64) {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut total_bytes: u64 = 0;
        let mut file_count: u64 = 0;
        if let Ok(walker) = walkdir(&root) {
            for entry in walker {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        total_bytes += meta.len();
                        file_count += 1;
                    }
                }
            }
        }
        (total_bytes, file_count)
    })
    .await
    .unwrap_or((0, 0))
}

/// Simple recursive directory walker (no external crate needed).
fn walkdir(root: &std::path::Path) -> std::io::Result<Vec<std::fs::DirEntry>> {
    let mut entries = Vec::new();
    walk_recursive(root, &mut entries)?;
    Ok(entries)
}

fn walk_recursive(
    dir: &std::path::Path,
    entries: &mut Vec<std::fs::DirEntry>,
) -> std::io::Result<()> {
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            entries.push(entry);
            let path = entries.last().unwrap().path();
            if path.is_dir() {
                walk_recursive(&path, entries)?;
            }
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/admin/diagnostics/config — read diagnostics config
// ═══════════════════════════════════════════════════════════════════════════

pub async fn get_diagnostics_config(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<DiagnosticsConfig>, AppError> {
    require_admin(&auth, &state).await?;

    let config = read_diagnostics_config(&state.pool).await;
    Ok(Json(config))
}

// ═══════════════════════════════════════════════════════════════════════════
// PUT /api/admin/diagnostics/config — update diagnostics config
// ═══════════════════════════════════════════════════════════════════════════

pub async fn update_diagnostics_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpdateDiagnosticsConfigRequest>,
) -> Result<Json<DiagnosticsConfig>, AppError> {
    require_admin(&auth, &state).await?;

    if let Some(enabled) = req.diagnostics_enabled {
        let val = if enabled { "true" } else { "false" };
        sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES ('diagnostics_enabled', ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(val)
        .execute(&state.pool)
        .await?;
    }

    if let Some(client_enabled) = req.client_diagnostics_enabled {
        let val = if client_enabled { "true" } else { "false" };
        sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES ('client_diagnostics_enabled', ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(val)
        .execute(&state.pool)
        .await?;
    }

    let config = read_diagnostics_config(&state.pool).await;
    Ok(Json(config))
}

/// Helper: read both diagnostics flags from server_settings.
async fn read_diagnostics_config(pool: &sqlx::SqlitePool) -> DiagnosticsConfig {
    let diag: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'diagnostics_enabled'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let client_diag: Option<String> = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'client_diagnostics_enabled'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    DiagnosticsConfig {
        diagnostics_enabled: diag.as_deref() == Some("true"),
        client_diagnostics_enabled: client_diag.as_deref() == Some("true"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/admin/diagnostics — comprehensive server metrics
// (returns lightweight stub when disabled to save performance)
// ═══════════════════════════════════════════════════════════════════════════

pub async fn get_diagnostics(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<axum::response::Response, AppError> {
    require_admin(&auth, &state).await?;

    let (start_instant, started_at) = server_start();
    let uptime = start_instant.elapsed().as_secs();

    // Check if diagnostics collection is enabled
    let config = read_diagnostics_config(&state.pool).await;

    if !config.diagnostics_enabled {
        // Return lightweight response — no expensive disk walks or table scans
        let resp = DisabledDiagnosticsResponse {
            enabled: false,
            server: BasicServerInfo {
                version: "0.6.9".to_string(),
                uptime_seconds: uptime,
                started_at: started_at.clone(),
            },
            message: "Diagnostics collection is disabled. Enable it to view full metrics.".into(),
        };
        return Ok(Json(resp).into_response());
    }

    // Parallelise independent DB queries
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

    // Count rows in key tables.
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

    let user_stats = UserStats {
        total_users,
        admin_count,
        totp_enabled_count,
    };

    // ── Photo stats ───────────────────────────────────────────────────
    let total_photos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let encrypted_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let plain_count = total_photos - encrypted_count;
    let total_file_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM photos")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    // No dedicated thumb_size column — estimate from count of photos with thumbnails
    let total_thumb_bytes: i64 = 0;
    let photos_with_thumbs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE thumb_path IS NOT NULL AND thumb_path != ''")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let favorited_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE is_favorite = 1")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let tagged_count: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT photo_id) FROM photo_tags")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let oldest_photo: Option<String> =
        sqlx::query_scalar("SELECT MIN(created_at) FROM photos")
            .fetch_one(pool)
            .await
            .unwrap_or(None);
    let newest_photo: Option<String> =
        sqlx::query_scalar("SELECT MAX(created_at) FROM photos")
            .fetch_one(pool)
            .await
            .unwrap_or(None);

    // Photos grouped by media_type
    let media_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT media_type, COUNT(*) as cnt FROM photos GROUP BY media_type",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let photos_by_media_type: HashMap<String, i64> =
        media_rows.into_iter().collect();

    let photo_stats = PhotoStats {
        total_photos,
        encrypted_count,
        plain_count,
        total_file_bytes,
        total_thumb_bytes,
        photos_with_thumbs,
        photos_by_media_type,
        oldest_photo,
        newest_photo,
        favorited_count,
        tagged_count,
    };

    // ── Audit summary ─────────────────────────────────────────────────
    let audit_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let now = chrono::Utc::now();
    let h24 = (now - chrono::Duration::hours(24)).to_rfc3339();
    let d7 = (now - chrono::Duration::days(7)).to_rfc3339();

    let audit_24h: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at > ?")
            .bind(&h24)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let audit_7d: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE created_at > ?")
            .bind(&d7)
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    let event_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT event_type, COUNT(*) as cnt FROM audit_log GROUP BY event_type",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let events_by_type: HashMap<String, i64> = event_rows.into_iter().collect();

    // Recent failures (last 50)
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
        .map(|(event_type, ip_address, user_agent, created_at, details)| {
            AuditFailureEntry {
                event_type,
                ip_address,
                user_agent,
                created_at,
                details,
            }
        })
        .collect();

    let audit_summary = AuditSummary {
        total_entries: audit_total,
        entries_last_24h: audit_24h,
        entries_last_7d: audit_7d,
        events_by_type,
        recent_failures,
    };

    // ── Client log summary ────────────────────────────────────────────
    let cl_total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM client_logs")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let cl_24h: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM client_logs WHERE created_at > ?")
            .bind(&h24)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let cl_7d: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM client_logs WHERE created_at > ?")
            .bind(&d7)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let cl_level_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT level, COUNT(*) as cnt FROM client_logs GROUP BY level",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let by_level: HashMap<String, i64> = cl_level_rows.into_iter().collect();
    let unique_sessions: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT session_id) FROM client_logs")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    let client_log_summary = ClientLogSummary {
        total_entries: cl_total,
        entries_last_24h: cl_24h,
        entries_last_7d: cl_7d,
        by_level,
        unique_sessions,
    };

    // ── Backup summary ────────────────────────────────────────────────
    let backup_servers: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM backup_servers")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let total_sync_logs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM backup_sync_log")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let last_sync_at: Option<String> =
        sqlx::query_scalar("SELECT MAX(started_at) FROM backup_sync_log")
            .fetch_one(pool)
            .await
            .unwrap_or(None);

    let backup_summary = BackupSummary {
        server_count: backup_servers,
        total_sync_logs,
        last_sync_at,
    };

    // ── Performance: DB ping ──────────────────────────────────────────
    let t0 = Instant::now();
    let _: i64 = sqlx::query_scalar("SELECT 1")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let db_ping_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // SQLite cache hit ratio from PRAGMA cache_stats (if available)
    let cache_hit_ratio: Option<f64> = None; // Not reliably available via sqlx

    let performance = PerformanceStats {
        db_ping_ms,
        cache_hit_ratio,
    };

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
// GET /api/admin/audit-logs — paginated audit log with filters
// ═══════════════════════════════════════════════════════════════════════════

pub async fn list_audit_logs(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<AuditLogParams>,
) -> Result<Json<AuditLogListResponse>, AppError> {
    require_admin(&auth, &state).await?;

    let limit = params.limit.unwrap_or(100).min(500) as i64;

    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref event_type) = params.event_type {
        conditions.push("a.event_type = ?".to_string());
        binds.push(event_type.clone());
    }
    if let Some(ref user_id) = params.user_id {
        conditions.push("a.user_id = ?".to_string());
        binds.push(user_id.clone());
    }
    if let Some(ref ip_address) = params.ip_address {
        conditions.push("a.ip_address = ?".to_string());
        binds.push(ip_address.clone());
    }
    if let Some(ref after) = params.after {
        conditions.push("a.created_at > ?".to_string());
        binds.push(after.clone());
    }
    if let Some(ref before) = params.before {
        conditions.push("a.created_at < ?".to_string());
        binds.push(before.clone());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    // Count total matching
    let count_sql = format!("SELECT COUNT(*) FROM audit_log a {}", where_clause);
    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in &binds {
        count_query = count_query.bind(b);
    }
    let total: i64 = count_query.fetch_one(&state.pool).await.unwrap_or(0);

    // Fetch rows with optional username join
    let sql = format!(
        "SELECT a.id, a.event_type, a.user_id, u.username, a.ip_address, a.user_agent, a.details, a.created_at \
         FROM audit_log a LEFT JOIN users u ON a.user_id = u.id {} \
         ORDER BY a.created_at DESC LIMIT ?",
        where_clause
    );

    let mut query =
        sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String, String, String, String)>(
            &sql,
        );
    for b in &binds {
        query = query.bind(b);
    }
    query = query.bind(limit + 1);

    let rows = query.fetch_all(&state.pool).await?;

    let has_more = rows.len() as i64 > limit;
    let entries: Vec<AuditLogEntry> = rows
        .into_iter()
        .take(limit as usize)
        .map(
            |(id, event_type, user_id, username, ip_address, user_agent, details, created_at)| {
                AuditLogEntry {
                    id,
                    event_type,
                    user_id,
                    username,
                    ip_address,
                    user_agent,
                    details,
                    created_at: created_at.clone(),
                }
            },
        )
        .collect();

    let next_cursor = if has_more {
        entries.last().map(|e| e.created_at.clone())
    } else {
        None
    };

    Ok(Json(AuditLogListResponse {
        logs: entries,
        next_cursor,
        total,
    }))
}
