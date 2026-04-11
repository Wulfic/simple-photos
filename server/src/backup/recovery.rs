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

use crate::audit::{self, AuditEvent};
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
    headers: HeaderMap,
    Path(server_id): Path<String>,
) -> Result<(StatusCode, Json<RecoveryResponse>), AppError> {
    require_admin(&state, &auth).await?;

    audit::log(
        &state,
        AuditEvent::RecoveryStart,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "server_id": server_id,
        })),
    )
    .await;

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
        AppError::Conflict("A sync or recovery is already in progress for this server".into())
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

    // Suppress auto-scan while recovery is in progress to avoid duplicate
    // photo entries from the scan and the incoming push running concurrently.
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('recovery_in_progress', 'true') \
         ON CONFLICT(key) DO UPDATE SET value = 'true'",
    )
    .execute(&state.pool)
    .await?;

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

        // NOTE: Do NOT delete restore_admin here — the backup hasn't pushed
        // real users yet, so user_count would drop to 0 and the frontend
        // would think setup is incomplete. The recovery_callback handler
        // deletes restore_admin after the backup finishes pushing users.

        // ── Phase 0: Pull user accounts directly from the backup ─────────
        // The push-sync engine will also sync users, but doing it here first
        // ensures accounts are available immediately (before photos arrive)
        // and avoids silent failures in the async push flow.
        let users_url = format!("{}/api/backup/list-users-full", backup_base);
        let mut users_req = client.get(&users_url);
        if let Some(ref key) = backup_api_key {
            users_req = users_req.header("X-API-Key", key);
        }

        let users_restored = match users_req.send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<Vec<serde_json::Value>>().await {
                    Ok(users) => {
                        let mut count = 0u32;
                        for user in &users {
                            if let Err(e) = upsert_user_from_backup(&pool, user).await {
                                tracing::warn!("Recovery: failed to upsert user: {}", e);
                            } else {
                                count += 1;
                            }
                        }
                        tracing::info!(
                            "Recovery from '{}': restored {} user account(s) directly",
                            server_name, count
                        );
                        count
                    }
                    Err(e) => {
                        tracing::warn!("Recovery: failed to parse user list: {}", e);
                        0
                    }
                }
            }
            Ok(resp) => {
                tracing::warn!(
                    "Recovery: list-users-full returned HTTP {}",
                    resp.status()
                );
                0
            }
            Err(e) => {
                tracing::warn!("Recovery: failed to fetch users from backup: {}", e);
                0
            }
        };

        // Now that real users exist, remove the temp restore_admin account
        if users_restored > 0 {
            let _ = sqlx::query("DELETE FROM users WHERE username = 'restore_admin'")
                .execute(&pool)
                .await;
            tracing::info!("Recovery: removed temporary restore_admin account");
        }

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

    // Insert a temporary row into backup_servers so the FK on
    // backup_sync_log is satisfied. It will be cleaned up after sync.
    sqlx::query(
        "INSERT INTO backup_servers (id, name, address, sync_frequency_hours, last_sync_status, enabled, created_at) \
         VALUES (?, ?, ?, 0, 'never', 0, ?)",
    )
    .bind(&temp_server.id)
    .bind(&temp_server.name)
    .bind(&temp_server.address)
    .bind(&temp_server.created_at)
    .execute(&state.pool)
    .await?;

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
        // Run the full sync engine targeting the recovering primary.
        // is_recovery=true skips the deletion phases that would otherwise
        // remove users/photos that exist on the primary but not on the backup.
        run_sync(&pool, &storage_root, &temp_server, &target_api_key, &log_id_clone, true).await;

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

        // Clean up temp server + sync log entries (CASCADE deletes logs)
        let _ = sqlx::query("DELETE FROM backup_servers WHERE id = ?")
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

    // Clear recovery flag so background auto-scan resumes, then run one
    // immediate scan to pick up any files on disk that the backup didn't have.
    let _ = sqlx::query("DELETE FROM server_settings WHERE key = 'recovery_in_progress'")
        .execute(&state.pool)
        .await;

    let storage_root = (**state.storage_root.load()).clone();
    let pool_clone = state.pool.clone();
    let jwt_secret = state.config.auth.jwt_secret.clone();
    tokio::spawn(async move {
        crate::backup::autoscan::run_auto_scan_public(&pool_clone, &storage_root).await;
        // Trigger encryption migration unconditionally: the backup may have
        // pushed photos whose encrypted_blob_id was still NULL (e.g. primary
        // encrypted them but the update was never synced back before crash).
        // Without this, the banner shows "encrypting N items" but migration
        // never runs (autoscan found 0 new files, so it doesn't trigger).
        crate::photos::server_migrate::auto_migrate_after_scan(
            pool_clone.clone(),
            storage_root,
            jwt_secret,
        )
        .await;
    });

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

/// Insert or update a user from the backup's list-users-full response.
///
/// Handles username conflicts by merging: if a local user (e.g. restore_admin)
/// has the same username, the local user is reparented and replaced.
async fn upsert_user_from_backup(
    pool: &sqlx::SqlitePool,
    user: &serde_json::Value,
) -> Result<(), String> {
    let id = user.get("id").and_then(|v| v.as_str())
        .ok_or("missing id")?;
    let username = user.get("username").and_then(|v| v.as_str())
        .ok_or("missing username")?;
    let password_hash = user.get("password_hash").and_then(|v| v.as_str())
        .ok_or("missing password_hash")?;
    let role = user.get("role").and_then(|v| v.as_str()).unwrap_or("user");
    let quota = user.get("storage_quota_bytes").and_then(|v| v.as_i64())
        .unwrap_or(10_737_418_240);
    let created_at = user.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
    let totp_secret: Option<&str> = user.get("totp_secret").and_then(|v| v.as_str());
    let totp_enabled: i32 = user.get("totp_enabled").and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;

    let result = sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, \
         role, totp_secret, totp_enabled) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
             username            = excluded.username, \
             password_hash       = excluded.password_hash, \
             role                = excluded.role, \
             storage_quota_bytes = excluded.storage_quota_bytes, \
             totp_secret         = excluded.totp_secret, \
             totp_enabled        = excluded.totp_enabled",
    )
    .bind(id)
    .bind(username)
    .bind(password_hash)
    .bind(created_at)
    .bind(quota)
    .bind(role)
    .bind(totp_secret)
    .bind(totp_enabled)
    .execute(pool)
    .await;

    if let Err(e) = result {
        let err_str = e.to_string();
        if err_str.contains("UNIQUE constraint failed: users.username") {
            // A local user with the same username but different ID exists
            // (e.g. restore_admin won't conflict, but if there's a real
            // collision we need to merge)
            let local_id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM users WHERE username = ? AND id != ?",
            )
            .bind(username)
            .bind(id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

            if let Some(ref old_id) = local_id {
                // Reparent content from old user to the backup's user ID
                for sql in &[
                    "UPDATE photos SET user_id = ? WHERE user_id = ?",
                    "UPDATE trash_items SET user_id = ? WHERE user_id = ?",
                    "UPDATE photo_tags SET user_id = ? WHERE user_id = ?",
                    "UPDATE blobs SET user_id = ? WHERE user_id = ?",
                    "UPDATE audit_log SET user_id = ? WHERE user_id = ?",
                    "UPDATE client_logs SET user_id = ? WHERE user_id = ?",
                    "UPDATE shared_albums SET owner_user_id = ? WHERE owner_user_id = ?",
                    "UPDATE shared_album_members SET user_id = ? WHERE user_id = ?",
                ] {
                    let _ = sqlx::query(sql).bind(id).bind(old_id).execute(pool).await;
                }

                let _ = sqlx::query("DELETE FROM encrypted_galleries WHERE user_id = ?")
                    .bind(old_id).execute(pool).await;
                let _ = sqlx::query("DELETE FROM users WHERE id = ?")
                    .bind(old_id).execute(pool).await;

                tracing::info!(
                    "Recovery: merged local user {} into backup user {} ({})",
                    old_id, id, username
                );
            }

            // Re-attempt insert now that the conflict is resolved
            sqlx::query(
                "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, \
                 role, totp_secret, totp_enabled) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET \
                     username            = excluded.username, \
                     password_hash       = excluded.password_hash, \
                     role                = excluded.role, \
                     storage_quota_bytes = excluded.storage_quota_bytes, \
                     totp_secret         = excluded.totp_secret, \
                     totp_enabled        = excluded.totp_enabled",
            )
            .bind(id)
            .bind(username)
            .bind(password_hash)
            .bind(created_at)
            .bind(quota)
            .bind(role)
            .bind(totp_secret)
            .bind(totp_enabled)
            .execute(pool)
            .await
            .map_err(|e| format!("Re-insert after merge failed: {}", e))?;
        } else {
            return Err(format!("DB error: {}", e));
        }
    }

    // Sync TOTP backup codes
    if let Some(codes) = user.get("totp_backup_codes").and_then(|v| v.as_array()) {
        let _ = sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
            .bind(id).execute(pool).await;

        for code in codes {
            let code_id = code.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let code_hash = code.get("code_hash").and_then(|v| v.as_str()).unwrap_or("");
            let used: i32 = code.get("used").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

            if !code_id.is_empty() && !code_hash.is_empty() {
                let _ = sqlx::query(
                    "INSERT INTO totp_backup_codes (id, user_id, code_hash, used) \
                     VALUES (?, ?, ?, ?) \
                     ON CONFLICT(id) DO UPDATE SET code_hash = excluded.code_hash, used = excluded.used",
                )
                .bind(code_id)
                .bind(id)
                .bind(code_hash)
                .bind(used)
                .execute(pool)
                .await;
            }
        }
    }

    Ok(())
}
