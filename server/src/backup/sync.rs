//! Backup sync HTTP handlers and background scheduler.
//!
//! This module contains the API endpoints that trigger syncs and the periodic
//! background task. The actual sync engine (delta-transfer, phases, counters)
//! lives in [`super::sync_engine`].
//!
//! **Concurrency lock** — a per-server guard prevents overlapping syncs
//! (manual `trigger_sync` vs. background `background_sync_task`).

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::*;
use super::sync_engine::run_sync;

// ── Concurrency Lock ─────────────────────────────────────────────────────────

/// Tracks which backup server IDs have an active sync in progress.
/// Prevents overlapping syncs to the same server (manual trigger vs. background).
fn active_syncs() -> &'static std::sync::Mutex<HashSet<String>> {
    static INSTANCE: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> =
        std::sync::OnceLock::new();
    INSTANCE.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

/// RAII guard that removes the server ID from the active set on drop.
pub struct SyncGuard {
    server_id: String,
}

impl Drop for SyncGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = active_syncs().lock() {
            set.remove(&self.server_id);
        }
    }
}

/// Try to acquire the sync lock for a server. Returns `Some(SyncGuard)` if
/// no other sync is running for that server, or `None` if one is already active.
pub fn try_acquire_sync(server_id: &str) -> Option<SyncGuard> {
    let mut set = active_syncs().lock().ok()?;
    if set.contains(server_id) {
        None
    } else {
        set.insert(server_id.to_string());
        Some(SyncGuard {
            server_id: server_id.to_string(),
        })
    }
}



// ── HTTP Handlers ────────────────────────────────────────────────────────────

/// POST /api/admin/backup/servers/:id/sync
/// Trigger an immediate sync to a backup server (admin only).
pub async fn trigger_sync(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(server_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    let server = sqlx::query_as::<_, BackupServer>(
        "SELECT id, name, address, sync_frequency_hours, last_sync_at, \
         last_sync_status, last_sync_error, enabled, created_at \
         FROM backup_servers WHERE id = ?",
    )
    .bind(&server_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    if !server.enabled {
        return Err(AppError::BadRequest("Backup server is disabled".into()));
    }

    // Prevent overlapping syncs to the same server
    let guard = try_acquire_sync(&server_id).ok_or_else(|| {
        AppError::BadRequest("A sync is already in progress for this server".into())
    })?;

    // Create sync log entry
    let log_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO backup_sync_log (id, server_id, started_at, status) VALUES (?, ?, ?, 'running')",
    )
    .bind(&log_id)
    .bind(&server_id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    // Spawn the sync as a background task
    let pool = state.pool.clone();
    let storage_root = (**state.storage_root.load()).clone();
    let api_key: Option<String> =
        sqlx::query_scalar("SELECT api_key FROM backup_servers WHERE id = ?")
            .bind(&server_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();

    let log_id_clone = log_id.clone();
    tokio::spawn(async move {
        // `guard` is moved into this future — the concurrency lock is held
        // for the entire duration of run_sync and released on drop.
        let _guard = guard;
        run_sync(&pool, &storage_root, &server, &api_key, &log_id_clone).await;
    });

    audit::log(
        &state,
        AuditEvent::SyncTrigger,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "server_id": server_id,
            "sync_id": log_id,
        })),
    )
    .await;

    Ok(Json(serde_json::json!({
        "message": "Sync started",
        "sync_id": log_id,
    })))
}

/// POST /api/backup/request-sync
/// Called by a backup server to request the primary to push data to it.
/// Authenticated via X-API-Key: the primary looks up the backup server by
/// matching the provided key against `backup_servers.api_key`.
pub async fn handle_request_sync(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let provided_key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing X-API-Key header".into()))?;

    // Look up which backup server this API key belongs to
    let server = sqlx::query_as::<_, BackupServer>(
        "SELECT id, name, address, sync_frequency_hours, last_sync_at, \
         last_sync_status, last_sync_error, enabled, created_at \
         FROM backup_servers WHERE api_key = ?",
    )
    .bind(provided_key)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Unknown API key".into()))?;

    if !server.enabled {
        return Err(AppError::BadRequest("Backup server is disabled".into()));
    }

    // Prevent overlapping syncs to the same server
    let guard = try_acquire_sync(&server.id).ok_or_else(|| {
        AppError::BadRequest("A sync is already in progress for this server".into())
    })?;

    // Create sync log entry
    let log_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO backup_sync_log (id, server_id, started_at, status) VALUES (?, ?, ?, 'running')",
    )
    .bind(&log_id)
    .bind(&server.id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    // Spawn the sync as a background task
    let pool = state.pool.clone();
    let storage_root = (**state.storage_root.load()).clone();
    let api_key: Option<String> = Some(provided_key.to_string());
    let log_id_clone = log_id.clone();
    let server_name = server.name.clone();

    tokio::spawn(async move {
        let _guard = guard;
        run_sync(&pool, &storage_root, &server, &api_key, &log_id_clone).await;
    });

    tracing::info!(
        server_name = %server_name,
        "Sync requested by backup server"
    );

    Ok(Json(serde_json::json!({
        "message": "Sync started",
        "sync_id": log_id,
    })))
}

/// POST /api/admin/backup/force-sync
/// Admin-only endpoint for backup servers. Contacts the primary server and
/// requests it to push the latest data to this backup instance.
pub async fn force_sync_from_primary(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    // This endpoint only makes sense on backup servers
    let mode: String =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_mode'")
            .fetch_optional(&state.read_pool)
            .await?
            .unwrap_or_else(|| "primary".to_string());

    if mode != "backup" {
        return Err(AppError::BadRequest(
            "This endpoint is only available on backup servers".into(),
        ));
    }

    // Get primary server URL and our API key
    let primary_url: String =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'primary_server_url'")
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or_else(|| AppError::BadRequest("No primary server URL configured".into()))?;

    let api_key: String = {
        // Prefer config-file key, fall back to DB-generated key
        if let Some(k) = state
            .config
            .backup
            .api_key
            .as_deref()
            .filter(|k| !k.is_empty())
        {
            k.to_string()
        } else {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
            )
            .fetch_optional(&state.read_pool)
            .await?
            .flatten()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| AppError::BadRequest("No backup API key configured".into()))?
        }
    };

    // Contact the primary server's request-sync endpoint
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    let url = format!(
        "{}/api/backup/request-sync",
        primary_url.trim_end_matches('/')
    );
    let resp = client
        .post(&url)
        .header("X-API-Key", &api_key)
        .send()
        .await
        .map_err(|e| {
            AppError::Internal(format!(
                "Failed to contact primary server at {}: {}",
                url, e
            ))
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "Primary server returned HTTP {}: {}",
            status,
            body.chars().take(500).collect::<String>()
        )));
    }

    let body: serde_json::Value = resp.json().await.unwrap_or_default();

    tracing::info!(
        "Force sync requested from primary server at {}",
        primary_url
    );

    audit::log(
        &state,
        AuditEvent::SyncForceFromPrimary,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "primary_url": primary_url,
        })),
    )
    .await;

    Ok(Json(serde_json::json!({
        "message": body.get("message").and_then(|m| m.as_str()).unwrap_or("Sync requested"),
        "sync_id": body.get("sync_id").and_then(|s| s.as_str()),
    })))
}

// ── Background Task ──────────────────────────────────────────────────────────

/// Background task: periodically sync to all enabled backup servers
/// based on their configured frequency.
pub async fn background_sync_task(pool: sqlx::SqlitePool, storage_root: std::path::PathBuf) {
    // Check every 5 minutes so newly-paired servers get synced quickly.
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));

    loop {
        interval.tick().await;

        let servers = match sqlx::query_as::<_, BackupServer>(
            "SELECT id, name, address, sync_frequency_hours, last_sync_at, \
             last_sync_status, last_sync_error, enabled, created_at \
             FROM backup_servers WHERE enabled = 1",
        )
        .fetch_all(&pool)
        .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to query backup servers: {}", e);
                continue;
            }
        };

        for server in &servers {
            // Retry policy:
            //  - Never synced (last_sync_at IS NULL) → sync immediately.
            //  - Last sync was "error" or "partial"  → retry after 1 h.
            //  - Last sync succeeded → wait sync_frequency_hours as usual.
            let should_sync = match &server.last_sync_at {
                None => true,
                Some(last) => {
                    if let Ok(last_dt) = chrono::DateTime::parse_from_rfc3339(last) {
                        let elapsed = Utc::now() - last_dt.with_timezone(&Utc);
                        let threshold_hours = match server.last_sync_status.as_str() {
                            "error" | "partial" => 1_i64,
                            _ => server.sync_frequency_hours,
                        };
                        elapsed.num_hours() >= threshold_hours
                    } else {
                        true
                    }
                }
            };

            if !should_sync {
                continue;
            }

            let guard = match try_acquire_sync(&server.id) {
                Some(g) => g,
                None => {
                    tracing::debug!(
                        server_id = %server.id,
                        "Skipping scheduled sync — another sync is already in progress"
                    );
                    continue;
                }
            };

            let log_id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();

            if let Err(e) = sqlx::query(
                "INSERT INTO backup_sync_log (id, server_id, started_at, status) \
                 VALUES (?, ?, ?, 'running')",
            )
            .bind(&log_id)
            .bind(&server.id)
            .bind(&now)
            .execute(&pool)
            .await
            {
                tracing::warn!(server_id = %server.id, error = %e, "Failed to create backup sync log entry");
            }

            let api_key: Option<String> =
                sqlx::query_scalar("SELECT api_key FROM backup_servers WHERE id = ?")
                    .bind(&server.id)
                    .fetch_optional(&pool)
                    .await
                    .ok()
                    .flatten();

            run_sync(&pool, &storage_root, server, &api_key, &log_id).await;

            audit::log_background(
                &pool,
                AuditEvent::BackupSyncCycleComplete,
                Some(serde_json::json!({
                    "server_id": server.id,
                    "server_name": server.name,
                    "sync_id": log_id,
                })),
            );

            drop(guard);
        }
    }
}
