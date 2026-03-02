use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::handlers::require_admin;
use super::models::*;

/// POST /api/admin/backup/servers/:id/sync
/// Trigger an immediate sync to a backup server (admin only).
pub async fn trigger_sync(
    State(state): State<AppState>,
    auth: AuthUser,
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
    let storage_root = state.storage_root.read().await.clone();
    let api_key: Option<String> = sqlx::query_scalar(
        "SELECT api_key FROM backup_servers WHERE id = ?",
    )
    .bind(&server_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();

    let log_id_clone = log_id.clone();
    tokio::spawn(async move {
        run_sync(&pool, &storage_root, &server, &api_key, &log_id_clone).await;
    });

    Ok(Json(serde_json::json!({
        "message": "Sync started",
        "sync_id": log_id,
    })))
}

// ── Sync Engine ──────────────────────────────────────────────────────────────

/// Run the actual sync operation against a backup server.
/// This sends all photos (including trash) to the backup server.
async fn run_sync(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
    server: &BackupServer,
    api_key: &Option<String>,
    log_id: &str,
) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            update_sync_log(pool, log_id, "error", 0, 0, Some(&e.to_string())).await;
            return;
        }
    };

    let base_url = format!("http://{}/api", server.address);
    let mut photos_synced = 0i64;
    let mut bytes_synced = 0i64;

    // Check whether audio files should be included in sync
    let audio_backup_enabled: bool = sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(false);

    // 1. Sync photos from the photos table
    // Conditionally exclude audio files based on the audio_backup_enabled setting
    let photos: Vec<(String, String, i64)> = {
        let query = if audio_backup_enabled {
            "SELECT id, file_path, size_bytes FROM photos"
        } else {
            "SELECT id, file_path, size_bytes FROM photos WHERE media_type != 'audio'"
        };
        match sqlx::query_as(query).fetch_all(pool).await {
            Ok(p) => p,
            Err(e) => {
                update_sync_log(pool, log_id, "error", 0, 0, Some(&e.to_string())).await;
                return;
            }
        }
    };

    for (photo_id, file_path, size) in &photos {
        let full_path = storage_root.join(file_path);
        if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
            continue;
        }

        let file_data = match tokio::fs::read(&full_path).await {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut req = client
            .post(format!("{}/backup/receive", base_url))
            .header("X-Photo-Id", photo_id.as_str())
            .header("X-File-Path", file_path.as_str())
            .header("X-Source", "photos")
            .body(file_data);

        if let Some(ref key) = api_key {
            req = req.header("X-API-Key", key.as_str());
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                photos_synced += 1;
                bytes_synced += size;
            }
            Ok(resp) => {
                tracing::warn!(
                    "Backup sync failed for photo {}: HTTP {}",
                    photo_id,
                    resp.status()
                );
            }
            Err(e) => {
                tracing::warn!("Backup sync failed for photo {}: {}", photo_id, e);
            }
        }
    }

    // 2. Sync trash items too — backup is an exact mirror
    let trash_items: Vec<(String, String, i64)> = match sqlx::query_as(
        "SELECT id, file_path, size_bytes FROM trash_items",
    )
    .fetch_all(pool)
    .await
    {
        Ok(t) => t,
        Err(e) => {
            update_sync_log(
                pool,
                log_id,
                "error",
                photos_synced,
                bytes_synced,
                Some(&e.to_string()),
            )
            .await;
            return;
        }
    };

    for (trash_id, file_path, size) in &trash_items {
        let full_path = storage_root.join(file_path);
        if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
            continue;
        }

        let file_data = match tokio::fs::read(&full_path).await {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut req = client
            .post(format!("{}/backup/receive", base_url))
            .header("X-Photo-Id", trash_id.as_str())
            .header("X-File-Path", file_path.as_str())
            .header("X-Source", "trash")
            .body(file_data);

        if let Some(ref key) = api_key {
            req = req.header("X-API-Key", key.as_str());
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                photos_synced += 1;
                bytes_synced += size;
            }
            _ => {}
        }
    }

    // Update sync log and server status
    update_sync_log(pool, log_id, "success", photos_synced, bytes_synced, None).await;

    let now = Utc::now().to_rfc3339();
    let _ = sqlx::query(
        "UPDATE backup_servers SET last_sync_at = ?, last_sync_status = 'success', \
         last_sync_error = NULL WHERE id = ?",
    )
    .bind(&now)
    .bind(&server.id)
    .execute(pool)
    .await;

    tracing::info!(
        "Backup sync to '{}' complete: {} photos, {} bytes",
        server.name,
        photos_synced,
        bytes_synced
    );
}

async fn update_sync_log(
    pool: &sqlx::SqlitePool,
    log_id: &str,
    status: &str,
    photos_synced: i64,
    bytes_synced: i64,
    error: Option<&str>,
) {
    let now = Utc::now().to_rfc3339();
    let _ = sqlx::query(
        "UPDATE backup_sync_log SET completed_at = ?, status = ?, photos_synced = ?, \
         bytes_synced = ?, error = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(status)
    .bind(photos_synced)
    .bind(bytes_synced)
    .bind(error)
    .bind(log_id)
    .execute(pool)
    .await;
}

/// Background task: periodically sync to all enabled backup servers
/// based on their configured frequency.
pub async fn background_sync_task(
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600)); // Check every hour

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
            // Check if it's time to sync
            let should_sync = match &server.last_sync_at {
                None => true, // Never synced
                Some(last) => {
                    if let Ok(last_dt) = chrono::DateTime::parse_from_rfc3339(last) {
                        let elapsed = Utc::now() - last_dt.with_timezone(&Utc);
                        elapsed.num_hours() >= server.sync_frequency_hours
                    } else {
                        true
                    }
                }
            };

            if !should_sync {
                continue;
            }

            let log_id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();

            let _ = sqlx::query(
                "INSERT INTO backup_sync_log (id, server_id, started_at, status) \
                 VALUES (?, ?, ?, 'running')",
            )
            .bind(&log_id)
            .bind(&server.id)
            .bind(&now)
            .execute(&pool)
            .await;

            let api_key: Option<String> = sqlx::query_scalar(
                "SELECT api_key FROM backup_servers WHERE id = ?",
            )
            .bind(&server.id)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();

            run_sync(&pool, &storage_root, server, &api_key, &log_id).await;
        }
    }
}
