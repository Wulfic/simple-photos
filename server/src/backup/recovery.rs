//! Disaster-recovery endpoints.
//!
//! Instead of a fragile pull-based recovery, we reuse the proven sync engine.
//! The recovering primary asks the backup server to **push** all its data
//! (users, photos, trash, tags, thumbnails) via the same mechanism used for
//! normal backup sync.
//!
//! Endpoints:
//! - `POST /api/admin/backup/servers/:id/recover` — triggers push-sync from backup
//! - `POST /api/backup/push-to` — runs local sync engine targeting a remote server
//! - `POST /api/backup/recovery-callback` — receives completion notification

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::*;
use super::recovery_engine::update_recovery_log;
use super::serve::validate_api_key;
use super::sync::try_acquire_sync;
use super::sync_engine::run_sync;

// ── Recovery ─────────────────────────────────────────────────────────────────

/// POST /api/admin/backup/servers/:id/recover
///
/// Restore data from a backup server by asking it to push all its data
/// to this primary using the proven sync engine. Handles users, photos,
/// trash, tags, and thumbnails — the exact same path as normal backup sync.
///
/// Flow:
/// 1. Generates a temporary API key so this server can accept incoming pushes
/// 2. Asks the backup to push all data via `POST /api/backup/push-to`
/// 3. The backup runs its sync engine targeting this server
/// 4. On completion, the backup calls back to update recovery status
pub async fn recover_from_backup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<(StatusCode, Json<RecoveryResponse>), AppError> {
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

    let backup_api_key: Option<String> =
        sqlx::query_scalar("SELECT api_key FROM backup_servers WHERE id = ?")
            .bind(&server_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();

    let guard = try_acquire_sync(&server_id).ok_or_else(|| {
        AppError::BadRequest("A sync or recovery is already in progress for this server".into())
    })?;

    // Generate a temporary API key so this primary can accept incoming pushes
    let recovery_api_key = Uuid::new_v4().to_string().replace('-', "");
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('backup_api_key', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&recovery_api_key)
    .execute(&state.pool)
    .await?;

    // Determine this server's routable address from base_url config
    let base_url_cfg = state.config.server.base_url.trim_end_matches('/').to_string();
    let primary_address = base_url_cfg
        .strip_prefix("https://")
        .or_else(|| base_url_cfg.strip_prefix("http://"))
        .unwrap_or(&base_url_cfg)
        .split('/')
        .next()
        .unwrap_or("localhost:8080")
        .to_string();

    let recovery_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let server_name = server.name.clone();
    let server_name_response = server_name.clone();

    sqlx::query(
        "INSERT INTO backup_sync_log (id, server_id, started_at, status) \
         VALUES (?, ?, ?, 'recovering')",
    )
    .bind(&recovery_id)
    .bind(&server_id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    // Build the backup server's HTTP base URL
    let backup_addr = server.address.trim().trim_end_matches('/');
    let backup_base = if backup_addr.starts_with("http://") || backup_addr.starts_with("https://") {
        backup_addr.to_string()
    } else {
        format!("http://{}", backup_addr)
    };

    let pool = state.pool.clone();
    let recovery_id_clone = recovery_id.clone();
    let callback_url = format!("{}/api/backup/recovery-callback", base_url_cfg);

    tokio::spawn(async move {
        let _guard = guard;

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .danger_accept_invalid_certs(true)
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                update_recovery_log(&pool, &recovery_id_clone, "error", 0, 0, Some(&e.to_string())).await;
                cleanup_recovery_key(&pool).await;
                return;
            }
        };

        // Delete the temp restore_admin user before sync pushes real users
        let _ = sqlx::query("DELETE FROM users WHERE username = 'restore_admin'")
            .execute(&pool)
            .await;

        // Ask the backup server to push all its data to this primary
        let push_url = format!("{}/api/backup/push-to", backup_base);
        let push_body = serde_json::json!({
            "target_address": primary_address,
            "target_api_key": recovery_api_key,
            "recovery_id": recovery_id_clone,
            "callback_url": callback_url,
        });

        let mut push_req = client.post(&push_url).json(&push_body);
        if let Some(ref key) = backup_api_key {
            push_req = push_req.header("X-API-Key", key);
        }

        match push_req.send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(
                    "Recovery push-sync triggered on backup '{}' → primary at {}",
                    server_name, primary_address
                );
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                let msg = format!("Backup server returned HTTP {}: {}", status, body);
                tracing::error!("Recovery push-sync failed: {}", msg);
                update_recovery_log(&pool, &recovery_id_clone, "error", 0, 0, Some(&msg)).await;
                cleanup_recovery_key(&pool).await;
            }
            Err(e) => {
                let msg = format!("Failed to contact backup server: {}", e);
                tracing::error!("Recovery push-sync failed: {}", msg);
                update_recovery_log(&pool, &recovery_id_clone, "error", 0, 0, Some(&msg)).await;
                cleanup_recovery_key(&pool).await;
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(RecoveryResponse {
            message: format!("Recovery from '{}' started", server_name_response),
            recovery_id,
        }),
    ))
}

// ── Push-Sync (called by recovering primary on the backup) ──────────────────

#[derive(Debug, Deserialize)]
pub struct PushSyncRequest {
    pub target_address: String,
    pub target_api_key: String,
    pub recovery_id: Option<String>,
    pub callback_url: Option<String>,
}

/// POST /api/backup/push-to
///
/// Push all local data (users, photos, trash, tags) to a specified target
/// server using the sync engine. Authenticated via X-API-Key.
///
/// Used by a recovering primary to ask this backup to push its data.
pub async fn push_sync_to_target(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PushSyncRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_api_key(&state, &headers).await?;

    let target_address = req.target_address.trim().to_string();
    if target_address.is_empty() {
        return Err(AppError::BadRequest("target_address is required".into()));
    }

    // Create a temporary BackupServer targeting the recovery primary
    let temp_server_id = format!("recovery-push-{}", Uuid::new_v4());
    let temp_server = BackupServer {
        id: temp_server_id.clone(),
        name: format!("Recovery target ({})", target_address),
        address: target_address,
        sync_frequency_hours: 0,
        last_sync_at: None,
        last_sync_status: "never".to_string(),
        last_sync_error: None,
        enabled: true,
        created_at: Utc::now().to_rfc3339(),
    };

    let log_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO backup_sync_log (id, server_id, started_at, status) VALUES (?, ?, ?, 'running')",
    )
    .bind(&log_id)
    .bind(&temp_server_id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    let pool = state.pool.clone();
    let storage_root = (**state.storage_root.load()).clone();
    let target_api_key = Some(req.target_api_key.clone());
    let log_id_clone = log_id.clone();
    let callback_url = req.callback_url;
    let recovery_id = req.recovery_id;
    let callback_api_key = req.target_api_key;

    tokio::spawn(async move {
        // Run the full sync engine targeting the recovering primary
        run_sync(&pool, &storage_root, &temp_server, &target_api_key, &log_id_clone).await;

        // Notify the primary of completion via callback
        if let Some(ref url) = callback_url {
            let (status, photos_synced, bytes_synced, error) =
                read_sync_log_result(&pool, &log_id_clone).await;

            if let Ok(c) = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .danger_accept_invalid_certs(true)
                .build()
            {
                let body = serde_json::json!({
                    "recovery_id": recovery_id,
                    "status": status,
                    "photos_synced": photos_synced,
                    "bytes_synced": bytes_synced,
                    "error": error,
                });
                let _ = c.post(url)
                    .header("X-API-Key", &callback_api_key)
                    .json(&body)
                    .send()
                    .await;
            }
        }

        // Clean up temp sync log entries
        let _ = sqlx::query("DELETE FROM backup_sync_log WHERE server_id = ?")
            .bind(&temp_server.id)
            .execute(&pool)
            .await;
    });

    Ok(Json(serde_json::json!({
        "message": "Push sync started",
        "sync_id": log_id,
    })))
}

// ── Recovery Callback ────────────────────────────────────────────────────────

/// POST /api/backup/recovery-callback
///
/// Called by the backup server after completing a push-sync. Updates the
/// recovery log entry and cleans up temporary settings.
pub async fn recovery_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let recovery_id = body.get("recovery_id").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing recovery_id".into()))?;
    let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("success");
    let photos_synced = body.get("photos_synced").and_then(|v| v.as_i64()).unwrap_or(0);
    let bytes_synced = body.get("bytes_synced").and_then(|v| v.as_i64()).unwrap_or(0);
    let error = body.get("error").and_then(|v| v.as_str());

    update_recovery_log(&state.pool, recovery_id, status, photos_synced, bytes_synced, error).await;

    tracing::info!(
        "Recovery complete: status={}, photos={}, bytes={}",
        status, photos_synced, bytes_synced
    );

    // Clean up: remove temp API key and temp restore_admin user
    cleanup_recovery_key(&state.pool).await;
    let _ = sqlx::query("DELETE FROM users WHERE username = 'restore_admin'")
        .execute(&state.pool)
        .await;

    Ok(StatusCode::OK)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Read the final sync result from backup_sync_log.
async fn read_sync_log_result(
    pool: &sqlx::SqlitePool,
    log_id: &str,
) -> (String, i64, i64, Option<String>) {
    match sqlx::query_as::<_, (String, Option<i64>, Option<i64>, Option<String>)>(
        "SELECT status, photos_synced, bytes_synced, error FROM backup_sync_log WHERE id = ?",
    )
    .bind(log_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some((status, photos, bytes, error))) => {
            (status, photos.unwrap_or(0), bytes.unwrap_or(0), error)
        }
        _ => ("unknown".to_string(), 0, 0, Some("Could not read sync log".to_string())),
    }
}

/// Remove the temporary backup_api_key from server_settings.
async fn cleanup_recovery_key(pool: &sqlx::SqlitePool) {
    let _ = sqlx::query("DELETE FROM server_settings WHERE key = 'backup_api_key'")
        .execute(pool)
        .await;
}
