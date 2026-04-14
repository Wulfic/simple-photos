//! One-way backup-server diagnostics.
//!
//! ## Data flow (primary only receives, never sends)
//!
//! ```text
//!   Backup server  ──POST /api/backup/report──►  Primary server
//!                        (X-API-Key auth)           stores in DB
//! ```
//!
//! **What runs where:**
//! - [`background_diagnostics_push_task`] — spawned on backup-mode servers;
//!   collects a health snapshot every 15 minutes and POSTs it to the primary.
//! - [`receive_backup_report`] — HTTP handler on the primary that accepts
//!   the incoming report and persists it in `backup_servers.last_diagnostics`.
//! - [`get_backup_diagnostics`] — admin handler that returns the latest stored
//!   report for a given backup server (admin Bearer JWT required).
//!
//! **Security:**
//! - The backup authenticates each push with `X-API-Key` (the same key used
//!   for all backup↔primary communication).
//! - Android clients and other non-backup callers cannot submit reports here.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::BackupDiagnosticsReport;
use super::models::{ForwardLogsRequest, ForwardLogsResponse};
use crate::state::AuditBroadcast;

// ── API-Key helper ────────────────────────────────────────────────────────────

/// Validate the `X-API-Key` header against the api_key column of all registered
/// backup servers on this (primary) instance. Returns the matching server's ID.
///
/// This is the *inverse* of the backup-serving validation in `serve.rs`:
/// instead of checking "does this key match MY api_key?", we check
/// "which backup_server row owns this key?".
async fn validate_backup_api_key(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, AppError> {
    let provided_key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing X-API-Key header".into()))?;

    // Look up the backup server row whose registered api_key matches
    let server_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM backup_servers WHERE api_key = ? AND enabled = 1")
            .bind(provided_key)
            .fetch_optional(&state.read_pool)
            .await?;

    server_id.ok_or_else(|| AppError::Unauthorized("Invalid or unknown backup API key".into()))
}

// ── Primary-side: receive a report ───────────────────────────────────────────

/// POST /api/backup/report
///
/// Receives a diagnostic health snapshot pushed by a backup server.
/// Authenticated exclusively via the `X-API-Key` header — no Bearer JWT.
///
/// The primary looks up which `backup_servers` row owns the presented key,
/// then persists the JSON report so admins can inspect it via the admin UI.
pub async fn receive_backup_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(report): Json<BackupDiagnosticsReport>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Resolve the sending backup server by its API key
    let server_id = validate_backup_api_key(&state, &headers).await?;

    let report_json = serde_json::to_string(&report)
        .map_err(|e| AppError::Internal(format!("Failed to serialise diagnostics: {}", e)))?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE backup_servers \
         SET last_diagnostics = ?, last_diagnostics_at = ? \
         WHERE id = ?",
    )
    .bind(&report_json)
    .bind(&now)
    .bind(&server_id)
    .execute(&state.pool)
    .await?;

    tracing::debug!(
        server_id = %server_id,
        photos = report.total_photos,
        disk_pct = report.disk_used_percent,
        "Stored diagnostics report from backup server"
    );

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ── Primary-side: receive forwarded audit logs ──────────────────────────────

/// POST /api/backup/forward-logs
///
/// Receives a batch of audit log entries pushed by a backup server.
/// Authenticated via `X-API-Key` — the same key used for all backup↔primary traffic.
/// Entries are stored in the primary's `audit_log` table with `source_server`
/// set to the backup server's name, so admins can see all events in one place.
pub async fn receive_forwarded_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ForwardLogsRequest>,
) -> Result<Json<ForwardLogsResponse>, AppError> {
    let server_id = validate_backup_api_key(&state, &headers).await?;

    // Look up backup server name for the source_server column
    let server_name: String = sqlx::query_scalar(
        "SELECT name FROM backup_servers WHERE id = ?",
    )
    .bind(&server_id)
    .fetch_optional(&state.read_pool)
    .await?
    .unwrap_or_else(|| server_id.clone());

    let count = payload.logs.len();
    if count == 0 {
        return Ok(Json(ForwardLogsResponse { accepted: 0 }));
    }

    // Cap batch size to prevent abuse
    let logs = if count > 1000 { &payload.logs[..1000] } else { &payload.logs };

    for entry in logs {
        // Use a prefixed ID to avoid collisions with local audit IDs
        let local_id = format!("fwd-{}", &entry.id);

        // Upsert — if this ID was already forwarded, skip it silently
        let result = sqlx::query(
            "INSERT OR IGNORE INTO audit_log \
             (id, event_type, user_id, ip_address, user_agent, details, created_at, source_server) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&local_id)
        .bind(&entry.event_type)
        .bind(&entry.user_id)
        .bind(&entry.ip_address)
        .bind(&entry.user_agent)
        .bind(&entry.details)
        .bind(&entry.created_at)
        .bind(&server_name)
        .execute(&state.pool)
        .await;

        match result {
            Ok(r) if r.rows_affected() > 0 => {
                // New entry — broadcast to SSE subscribers
                let _ = state.audit_tx.send(AuditBroadcast {
                    id: local_id,
                    event_type: entry.event_type.clone(),
                    user_id: entry.user_id.clone(),
                    ip_address: entry.ip_address.clone(),
                    user_agent: entry.user_agent.clone(),
                    details: entry.details.clone(),
                    created_at: entry.created_at.clone(),
                    source_server: Some(server_name.clone()),
                });
            }
            Ok(_) => {} // Already forwarded — skip
            Err(e) => {
                tracing::warn!(
                    server = %server_name,
                    entry_id = %entry.id,
                    error = %e,
                    "Failed to store forwarded audit log entry"
                );
            }
        }
    }

    tracing::debug!(
        server = %server_name,
        count = logs.len(),
        "Stored forwarded audit logs from backup server"
    );

    Ok(Json(ForwardLogsResponse { accepted: logs.len() }))
}

// ── Primary-side: read the latest report ────────────────────────────────────

/// GET /api/admin/backup/servers/:id/diagnostics
///
/// Returns the most recent diagnostics report received from a backup server,
/// or 404 if no report has been received yet. Admin Bearer JWT required.
pub async fn get_backup_diagnostics(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    // Returns None if the server ID doesn't exist; the inner Option fields
    // are None if no report has been received yet.
    let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT last_diagnostics, last_diagnostics_at FROM backup_servers WHERE id = ?",
    )
    .bind(&server_id)
    .fetch_optional(&state.read_pool)
    .await?;

    // Outer None → server row not found; inner None → row exists but no report yet
    let (json_str, received_at) = row.ok_or(AppError::NotFound)?;

    match json_str {
        Some(s) => {
            let data: serde_json::Value =
                serde_json::from_str(&s).unwrap_or(serde_json::Value::Null);
            Ok(Json(serde_json::json!({
                "received_at": received_at,
                "report": data,
            })))
        }
        None => Err(AppError::NotFound),
    }
}

// ── Backup-side: periodic push task ─────────────────────────────────────────

/// Background task running on backup-mode servers.
///
/// Every 15 minutes, collects a lightweight health snapshot and POSTs it
/// to `{primary_server_url}/api/backup/report` using the backup API key.
/// The task silently skips cycles when the primary is unreachable — the
/// next cycle will retry.
///
/// The task is only meaningful when the server is in backup mode; it
/// checks the database setting on every tick so it naturally becomes
/// dormant if the server is ever switched back to primary mode.
pub async fn background_diagnostics_push_task(
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
    db_path: std::path::PathBuf,
    accept_invalid_certs: bool,
) {
    use crate::diagnostics::handlers::{
        disk_stats, read_cpu_seconds, read_rss_bytes, server_start,
    };

    // Push every 15 minutes — frequent enough to be useful for monitoring
    // but lean enough not to add meaningful load.
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(900));

    loop {
        interval.tick().await;

        // Only run when this server is in backup mode
        let mode: String =
            match sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_mode'")
                .fetch_optional(&pool)
                .await
            {
                Ok(Some(m)) => m,
                _ => "primary".to_string(),
            };

        if mode != "backup" {
            continue;
        }

        // Retrieve configuration stored during pairing
        let primary_url: Option<String> = sqlx::query_scalar(
            "SELECT value FROM server_settings WHERE key = 'primary_server_url'",
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();

        let api_key: Option<String> =
            sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_api_key'")
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();

        let (primary_url, api_key) = match (primary_url, api_key) {
            (Some(u), Some(k)) if !u.is_empty() && !k.is_empty() => (u, k),
            _ => {
                tracing::debug!(
                    "Diagnostics push: primary_server_url or backup_api_key not set — skipping"
                );
                continue;
            }
        };

        // ── Collect health metrics ────────────────────────────────────
        let (start_instant, _) = server_start();
        let uptime_seconds = start_instant.elapsed().as_secs();

        let (rss_bytes, cpu_secs) =
            tokio::task::spawn_blocking(|| (read_rss_bytes(), read_cpu_seconds()))
                .await
                .unwrap_or((0, 0.0));

        let total_photos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap_or(0);

        let (disk_total, disk_available) = tokio::task::spawn_blocking({
            let p = storage_root.clone();
            move || disk_stats(&p)
        })
        .await
        .unwrap_or((0, 0));
        let disk_used_percent = if disk_total > 0 {
            ((disk_total - disk_available) as f64 / disk_total as f64) * 100.0
        } else {
            0.0
        };

        let db_size_bytes = tokio::fs::metadata(&db_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let report = BackupDiagnosticsReport {
            version: crate::VERSION.to_string(),
            uptime_seconds,
            memory_rss_bytes: rss_bytes,
            cpu_seconds: cpu_secs,
            total_photos,
            disk_used_percent,
            db_size_bytes,
            collected_at: Utc::now().to_rfc3339(),
        };

        // ── Push to primary ───────────────────────────────────────────
        let url = format!("{}/api/backup/report", primary_url.trim_end_matches('/'));

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .danger_accept_invalid_certs(accept_invalid_certs)
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Diagnostics push: failed to build HTTP client: {}", e);
                continue;
            }
        };

        match client
            .post(&url)
            .header("X-API-Key", &api_key)
            .header("Content-Type", "application/json")
            .json(&report)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(
                    primary = %primary_url,
                    photos = total_photos,
                    disk_pct = disk_used_percent,
                    "Pushed diagnostics report to primary"
                );
            }
            Ok(resp) => {
                tracing::warn!(
                    primary = %primary_url,
                    status = %resp.status(),
                    "Diagnostics push rejected by primary"
                );
            }
            Err(e) => {
                tracing::debug!(
                    primary = %primary_url,
                    error = %e,
                    "Diagnostics push failed — will retry next cycle"
                );
            }
        }
    }
}

// ── Backup-side: real-time audit log forwarding ─────────────────────────────

/// Background task running on backup-mode servers.
///
/// Subscribes to the local audit broadcast channel and forwards each new
/// entry to the primary immediately. Every 60 seconds, sweeps the DB for any
/// entries missed by the broadcast (e.g. due to lag) and forwards those too.
///
/// Only active when the server is in backup mode.
pub async fn background_log_forward_task(
    pool: sqlx::SqlitePool,
    audit_tx: tokio::sync::broadcast::Sender<AuditBroadcast>,
    accept_invalid_certs: bool,
) {
    use super::models::ForwardedAuditLog;

    let mut rx = audit_tx.subscribe();
    // Sweep every 60s as a safety net for missed entries
    let mut sweep_interval = tokio::time::interval(std::time::Duration::from_secs(60));
    // Buffer for batching real-time entries (flush after short delay or when full)
    let mut pending: Vec<ForwardedAuditLog> = Vec::new();
    let flush_delay = std::time::Duration::from_millis(500);

    loop {
        // Only run in backup mode
        let mode: String =
            match sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_mode'")
                .fetch_optional(&pool)
                .await
            {
                Ok(Some(m)) => m,
                _ => "primary".to_string(),
            };

        if mode != "backup" {
            // Not in backup mode — drain broadcast and sleep before rechecking
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            // Drain any accumulated messages to avoid lag errors
            while rx.try_recv().is_ok() {}
            continue;
        }

        let primary_url: Option<String> = sqlx::query_scalar(
            "SELECT value FROM server_settings WHERE key = 'primary_server_url'",
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();

        let api_key: Option<String> =
            sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_api_key'")
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();

        let (primary_url, api_key) = match (primary_url, api_key) {
            (Some(u), Some(k)) if !u.is_empty() && !k.is_empty() => (u, k),
            _ => {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                while rx.try_recv().is_ok() {}
                continue;
            }
        };

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .danger_accept_invalid_certs(accept_invalid_certs)
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Log forward: failed to build HTTP client: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
        };

        // Inner event loop — stays here while we're in backup mode
        loop {
            tokio::select! {
                // Real-time broadcast entry
                msg = rx.recv() => {
                    match msg {
                        Ok(entry) if entry.source_server.is_none() => {
                            pending.push(ForwardedAuditLog {
                                id: entry.id,
                                event_type: entry.event_type,
                                user_id: entry.user_id,
                                ip_address: entry.ip_address,
                                user_agent: entry.user_agent,
                                details: entry.details,
                                created_at: entry.created_at,
                            });
                            // If we have a small batch, wait briefly for more
                            if pending.len() < 10 {
                                tokio::time::sleep(flush_delay).await;
                                // Drain any more that arrived during the delay
                                while let Ok(e) = rx.try_recv() {
                                    if e.source_server.is_none() {
                                        pending.push(ForwardedAuditLog {
                                            id: e.id,
                                            event_type: e.event_type,
                                            user_id: e.user_id,
                                            ip_address: e.ip_address,
                                            user_agent: e.user_agent,
                                            details: e.details,
                                            created_at: e.created_at,
                                        });
                                    }
                                }
                            }
                        }
                        Ok(_) => continue, // Forwarded entry — skip
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // We'll catch up on the next sweep
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    }
                }
                // Periodic sweep for missed entries
                _ = sweep_interval.tick() => {
                    let sweep_entries = fetch_unsent_logs(&pool).await;
                    for entry in sweep_entries {
                        // Dedup: don't re-add if already pending
                        if !pending.iter().any(|p| p.id == entry.id) {
                            pending.push(entry);
                        }
                    }
                }
            }

            if pending.is_empty() {
                continue;
            }

            // Flush pending to primary
            let url = format!(
                "{}/api/backup/forward-logs",
                primary_url.trim_end_matches('/')
            );

            let batch: Vec<ForwardedAuditLog> = pending.drain(..).collect();
            let last_created_at = batch.last().map(|e| e.created_at.clone());

            match client
                .post(&url)
                .header("X-API-Key", &api_key)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "logs": batch }))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Some(ts) = last_created_at {
                        let _ = sqlx::query(
                            "INSERT INTO server_settings (key, value) \
                             VALUES ('last_forwarded_log_at', ?) \
                             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        )
                        .bind(&ts)
                        .execute(&pool)
                        .await;
                    }

                    tracing::debug!(
                        primary = %primary_url,
                        count = batch.len(),
                        "Forwarded audit logs to primary (real-time)"
                    );
                }
                Ok(resp) => {
                    tracing::warn!(
                        primary = %primary_url,
                        status = %resp.status(),
                        "Log forward rejected by primary"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        primary = %primary_url,
                        error = %e,
                        "Log forward failed — will retry on next event"
                    );
                }
            }

            // Re-check mode periodically — break inner loop to re-evaluate
            let mode: String = sqlx::query_scalar(
                "SELECT value FROM server_settings WHERE key = 'backup_mode'",
            )
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "primary".to_string());

            if mode != "backup" {
                break;
            }
        }
    }
}

/// Fetch local audit log entries that haven't been forwarded yet (by cursor).
async fn fetch_unsent_logs(
    pool: &sqlx::SqlitePool,
) -> Vec<super::models::ForwardedAuditLog> {
    use super::models::ForwardedAuditLog;

    let last_forwarded: Option<String> = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'last_forwarded_log_at'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let rows: Vec<(String, String, Option<String>, String, String, String, String)> =
        if let Some(ref cursor) = last_forwarded {
            sqlx::query_as(
                "SELECT id, event_type, user_id, ip_address, user_agent, details, created_at \
                 FROM audit_log \
                 WHERE source_server IS NULL AND created_at > ? \
                 ORDER BY created_at ASC \
                 LIMIT 500",
            )
            .bind(cursor)
            .fetch_all(pool)
            .await
            .unwrap_or_default()
        } else {
            let cutoff = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
            sqlx::query_as(
                "SELECT id, event_type, user_id, ip_address, user_agent, details, created_at \
                 FROM audit_log \
                 WHERE source_server IS NULL AND created_at > ? \
                 ORDER BY created_at ASC \
                 LIMIT 500",
            )
            .bind(&cutoff)
            .fetch_all(pool)
            .await
            .unwrap_or_default()
        };

    rows.into_iter()
        .map(|(id, event_type, user_id, ip_address, user_agent, details, created_at)| {
            ForwardedAuditLog {
                id,
                event_type,
                user_id,
                ip_address,
                user_agent,
                details,
                created_at,
            }
        })
        .collect()
}
