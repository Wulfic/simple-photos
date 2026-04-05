//! Server diagnostics and audit log endpoints (admin only).
//!
//! Provides system metrics (CPU, memory, disk, DB size, photo/blob counts),
//! diagnostics enable/disable configuration, and paginated audit log viewing.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use std::time::Instant;
use futures_util::stream::Stream;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::*;

/// Lazily initialised server start time – set once on first diagnostics call.
static SERVER_START: std::sync::OnceLock<(Instant, String)> = std::sync::OnceLock::new();

pub(crate) fn server_start() -> &'static (Instant, String) {
    SERVER_START.get_or_init(|| (Instant::now(), chrono::Utc::now().to_rfc3339()))
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

/// Read the number of threads from `/proc/self/status` on Linux.
#[cfg(target_os = "linux")]
pub(crate) fn read_thread_count() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Threads:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_thread_count() -> u64 {
    0
}

/// Count open file descriptors from `/proc/self/fd` on Linux.
#[cfg(target_os = "linux")]
pub(crate) fn read_open_fds() -> u64 {
    std::fs::read_dir("/proc/self/fd")
        .map(|entries| entries.count() as u64)
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_open_fds() -> u64 {
    0
}

/// Read system load averages (1min, 5min, 15min).
#[cfg(unix)]
pub(crate) fn read_load_average() -> [f64; 3] {
    let mut info: libc::sysinfo = unsafe { std::mem::zeroed() };
    if unsafe { libc::sysinfo(&mut info) } == 0 {
        let scale = 1.0 / (1 << libc::SI_LOAD_SHIFT) as f64;
        [
            info.loads[0] as f64 * scale,
            info.loads[1] as f64 * scale,
            info.loads[2] as f64 * scale,
        ]
    } else {
        [0.0, 0.0, 0.0]
    }
}

#[cfg(not(unix))]
pub(crate) fn read_load_average() -> [f64; 3] {
    [0.0, 0.0, 0.0]
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
            let path = entry.path();
            entries.push(entry);
            if path.is_dir() {
                walk_recursive(&path, entries)?;
            }
        }
    }
    Ok(())
}

/// GET /api/admin/diagnostics/config — read diagnostics configuration.
pub async fn get_diagnostics_config(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<DiagnosticsConfig>, AppError> {
    require_admin(&state, &auth).await?;

    let config = read_diagnostics_config(&state.read_pool).await;
    Ok(Json(config))
}

/// PUT /api/admin/diagnostics/config — update diagnostics configuration.
pub async fn update_diagnostics_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpdateDiagnosticsConfigRequest>,
) -> Result<Json<DiagnosticsConfig>, AppError> {
    require_admin(&state, &auth).await?;

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

    let config = read_diagnostics_config(&state.read_pool).await;
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

/// GET /api/admin/diagnostics — comprehensive server metrics.
///
/// Returns a lightweight stub when diagnostics collection is disabled
/// to avoid expensive disk walks and table scans.
pub async fn get_diagnostics(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<axum::response::Response, AppError> {
    require_admin(&state, &auth).await?;

    let (start_instant, started_at) = server_start();
    let uptime = start_instant.elapsed().as_secs();

    // Check if diagnostics collection is enabled
    let config = read_diagnostics_config(&state.read_pool).await;

    if !config.diagnostics_enabled {
        // Return lightweight response — no expensive disk walks or table scans
        let resp = DisabledDiagnosticsResponse {
            enabled: false,
            server: BasicServerInfo {
                version: crate::VERSION.to_string(),
                uptime_seconds: uptime,
                started_at: started_at.clone(),
            },
            message: "Diagnostics collection is disabled. Enable it to view full metrics.".into(),
        };
        return Ok(Json(resp).into_response());
    }

    let storage_root = (**state.storage_root.load()).clone();
    let resp = super::collect::collect_full_diagnostics(
        &state.read_pool,
        &state.pool,
        &state.config,
        &storage_root,
    )
    .await;

    Ok(Json(resp).into_response())
}

/// GET /api/admin/audit-logs — paginated audit log with optional filters.
pub async fn list_audit_logs(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<AuditLogParams>,
) -> Result<Json<AuditLogListResponse>, AppError> {
    require_admin(&state, &auth).await?;

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
    if let Some(ref source) = params.source_server {
        if source == "local" {
            conditions.push("a.source_server IS NULL".to_string());
        } else {
            conditions.push("a.source_server = ?".to_string());
            binds.push(source.clone());
        }
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
    let total: i64 = count_query.fetch_one(&state.read_pool).await.unwrap_or(0);

    // Fetch rows with optional username join
    let sql = format!(
        "SELECT a.id, a.event_type, a.user_id, u.username, a.ip_address, a.user_agent, a.details, a.created_at, a.source_server \
         FROM audit_log a LEFT JOIN users u ON a.user_id = u.id {} \
         ORDER BY a.created_at DESC LIMIT ?",
        where_clause
    );

    let mut query = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            String,
            String,
            Option<String>,
        ),
    >(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    query = query.bind(limit + 1);

    let rows = query.fetch_all(&state.read_pool).await?;

    let has_more = rows.len() as i64 > limit;
    let entries: Vec<AuditLogEntry> = rows
        .into_iter()
        .take(limit as usize)
        .map(
            |(id, event_type, user_id, username, ip_address, user_agent, details, created_at, source_server)| {
                AuditLogEntry {
                    id,
                    event_type,
                    user_id,
                    username,
                    ip_address,
                    user_agent,
                    details,
                    created_at,
                    source_server,
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

/// GET /api/admin/audit-logs/stream?token=<jwt>
///
/// Server-Sent Events endpoint that streams new audit log entries in real time.
/// Uses `?token=<jwt>` for authentication since EventSource cannot set headers.
/// Admin-only — rejects non-admin users.
pub async fn stream_audit_logs(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, AppError> {
    require_admin(&state, &auth).await?;

    let mut rx = state.audit_tx.subscribe();

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(entry) => {
                    if let Ok(json) = serde_json::to_string(&entry) {
                        yield Ok(Event::default().data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // Subscriber fell behind — notify the client
                    let msg = serde_json::json!({ "lagged": n });
                    yield Ok(Event::default().event("lagged").data(msg.to_string()));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
