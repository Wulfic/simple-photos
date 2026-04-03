//! Backup mode toggling (primary/backup) and audio backup settings.

use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::broadcast;
use super::models::*;

// ── Backup Mode Endpoints ────────────────────────────────────────────────────

/// GET /api/admin/backup/mode
/// Returns the current server mode ("primary" or "backup") and the server's local IP.
pub async fn get_backup_mode(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<BackupModeResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let mode: String =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_mode'")
            .fetch_optional(&state.read_pool)
            .await?
            .unwrap_or_else(|| "primary".to_string());

    let local_ip = broadcast::get_local_ip().unwrap_or_else(|| "unknown".to_string());
    let port = state.config.server.port;

    // Include the API key and primary server URL when in backup mode
    let (api_key, primary_server_url) = if mode == "backup" {
        let key = sqlx::query_scalar::<_, Option<String>>(
            "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
        )
        .fetch_optional(&state.read_pool)
        .await?
        .flatten();

        let primary_url: Option<String> = sqlx::query_scalar(
            "SELECT value FROM server_settings WHERE key = 'primary_server_url'",
        )
        .fetch_optional(&state.read_pool)
        .await?;

        (key, primary_url)
    } else {
        (None, None)
    };

    Ok(Json(BackupModeResponse {
        mode,
        server_ip: local_ip.clone(),
        server_address: format!("{}:{}", local_ip, port),
        port,
        api_key,
        primary_server_url,
    }))
}

/// POST /api/admin/backup/mode
/// Set the server mode to "primary" or "backup".
/// When set to "backup", the server broadcasts its presence on the LAN for discovery.
pub async fn set_backup_mode(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SetBackupModeRequest>,
) -> Result<Json<BackupModeResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let mode = match req.mode.as_str() {
        "primary" | "backup" => req.mode.clone(),
        _ => {
            return Err(AppError::BadRequest(
                "Mode must be 'primary' or 'backup'".into(),
            ))
        }
    };

    // Upsert the backup_mode setting
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('backup_mode', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&mode)
    .execute(&state.pool)
    .await?;

    // If enabling backup mode, auto-generate an API key if none is configured
    if mode == "backup" {
        let has_key = state
            .config
            .backup
            .api_key
            .as_deref()
            .filter(|k| !k.is_empty())
            .is_some();

        if !has_key {
            let key = Uuid::new_v4().to_string().replace('-', "");
            // Store in server_settings for persistence
            sqlx::query(
                "INSERT INTO server_settings (key, value) VALUES ('backup_api_key', ?) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(&key)
            .execute(&state.pool)
            .await?;

            tracing::info!("Generated backup API key for backup mode");
        }
    }

    let local_ip = broadcast::get_local_ip().unwrap_or_else(|| "unknown".to_string());
    let port = state.config.server.port;

    tracing::info!("Server mode set to '{}'", mode);

    // Include the API key and primary server URL when in backup mode
    let (api_key_val, primary_server_url): (Option<String>, Option<String>) = if mode == "backup" {
        let key = sqlx::query_scalar::<_, Option<String>>(
            "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
        )
        .fetch_optional(&state.read_pool)
        .await?
        .flatten();

        let primary_url: Option<String> = sqlx::query_scalar(
            "SELECT value FROM server_settings WHERE key = 'primary_server_url'",
        )
        .fetch_optional(&state.read_pool)
        .await?;

        (key, primary_url)
    } else {
        (None, None)
    };

    Ok(Json(BackupModeResponse {
        mode,
        server_ip: local_ip.clone(),
        server_address: format!("{}:{}", local_ip, port),
        port,
        api_key: api_key_val,
        primary_server_url,
    }))
}

/// GET /api/admin/backup/servers/:id/logs
/// Get sync history for a backup server (admin only).
pub async fn get_sync_logs(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<Json<Vec<SyncLogEntry>>, AppError> {
    require_admin(&state, &auth).await?;

    let logs = sqlx::query_as::<_, SyncLogEntry>(
        "SELECT id, server_id, started_at, completed_at, status, photos_synced, \
         bytes_synced, error FROM backup_sync_log \
         WHERE server_id = ? ORDER BY started_at DESC LIMIT 50",
    )
    .bind(&server_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(logs))
}

// ── Audio Backup Setting ─────────────────────────────────────────────────────

/// GET /api/settings/audio-backup
/// Returns whether audio files are included in backup sync.
pub async fn get_audio_backup_setting(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let enabled: String =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'audio_backup_enabled'")
            .fetch_optional(&state.read_pool)
            .await?
            .unwrap_or_else(|| "false".to_string());

    Ok(Json(serde_json::json!({
        "audio_backup_enabled": enabled == "true",
    })))
}

/// PUT /api/admin/audio-backup
/// Toggle whether audio files are included in backup sync (admin only).
pub async fn set_audio_backup_setting(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    let enabled = body
        .get("audio_backup_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('audio_backup_enabled', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(if enabled { "true" } else { "false" })
    .execute(&state.pool)
    .await?;

    tracing::info!("Audio backup setting updated: enabled={}", enabled);

    Ok(Json(serde_json::json!({
        "audio_backup_enabled": enabled,
        "message": if enabled {
            "Audio files will be included in backups."
        } else {
            "Audio files will be excluded from backups."
        },
    })))
}
