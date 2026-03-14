//! Backup sync engine — pushes local photos + trash to a remote backup
//! server via `POST /api/backup/receive`.
//!
//! **Features:**
//! - **Delta sync** — queries the remote's existing photo/trash IDs before
//!   transferring, so only new or changed items cross the network.
//! - **Concurrency lock** — a per-server guard prevents overlapping syncs
//!   (manual `trigger_sync` vs. background `background_sync_task`).
//! - **Checksum verification** — each transfer includes an `X-Content-Hash`
//!   (SHA-256) header so the receiver can verify data integrity.
//! - **Per-file error tracking** — individual failures are counted and the
//!   sync log records `"partial"` status with error details when some
//!   transfers fail.
//!
//! **Remaining limitation:**
//! - **Plain HTTP** — the API key is sent in cleartext unless a TLS-terminating
//!   proxy sits between the two servers.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::*;

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
        // `guard` is moved into this future — the concurrency lock is held
        // for the entire duration of run_sync and released on drop.
        let _guard = guard;
        run_sync(&pool, &storage_root, &server, &api_key, &log_id_clone).await;
    });

    Ok(Json(serde_json::json!({
        "message": "Sync started",
        "sync_id": log_id,
    })))
}

// ── Sync Engine ──────────────────────────────────────────────────────────────

/// Run the actual sync operation against a backup server.
///
/// **Delta sync:** queries the remote's existing photo & trash IDs first, then
/// only transfers items the remote doesn't already have.
///
/// **Checksums:** each file is sent with an `X-Content-Hash` (hex-encoded
/// SHA-256) header so the receiver can verify data integrity.
///
/// **Per-file tracking:** individual successes and failures are counted. The
/// sync log records `"partial"` when some transfers fail, or `"success"` when
/// all succeed.
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
    let mut failures = 0i64;
    let mut last_error: Option<String> = None;

    // ── Delta: fetch IDs the remote already has ──────────────────────────
    let remote_photo_ids: HashSet<String> =
        fetch_remote_ids(&client, &base_url, "/backup/list", api_key).await;
    let remote_trash_ids: HashSet<String> =
        fetch_remote_ids(&client, &base_url, "/backup/list-trash", api_key).await;

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

    // 1. Sync photos — only those the remote doesn't have yet
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

    let photos_to_sync: Vec<_> = photos
        .iter()
        .filter(|(id, _, _)| !remote_photo_ids.contains(id))
        .collect();

    tracing::info!(
        "Backup sync to '{}': {}/{} photos need transfer (delta)",
        server.name,
        photos_to_sync.len(),
        photos.len()
    );

    for (photo_id, file_path, size) in &photos_to_sync {
        match send_file(
            &client, &base_url, api_key, storage_root, photo_id, file_path, "photos",
        )
        .await
        {
            Ok(()) => {
                photos_synced += 1;
                bytes_synced += *size;
            }
            Err(e) => {
                failures += 1;
                last_error = Some(e.clone());
                tracing::warn!("Backup sync failed for photo {}: {}", photo_id, e);
            }
        }
    }

    // 2. Sync trash items — only those the remote doesn't have yet
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

    let trash_to_sync: Vec<_> = trash_items
        .iter()
        .filter(|(id, _, _)| !remote_trash_ids.contains(id))
        .collect();

    tracing::info!(
        "Backup sync to '{}': {}/{} trash items need transfer (delta)",
        server.name,
        trash_to_sync.len(),
        trash_items.len()
    );

    for (trash_id, file_path, size) in &trash_to_sync {
        match send_file(
            &client, &base_url, api_key, storage_root, trash_id, file_path, "trash",
        )
        .await
        {
            Ok(()) => {
                photos_synced += 1;
                bytes_synced += *size;
            }
            Err(e) => {
                failures += 1;
                last_error = Some(e.clone());
                tracing::warn!("Backup sync failed for trash {}: {}", trash_id, e);
            }
        }
    }

    // 3. Determine final status based on failure count
    let (status, error_detail) = if failures == 0 {
        ("success", None)
    } else if photos_synced == 0 && failures > 0 {
        (
            "error",
            Some(format!(
                "All {} transfers failed. Last error: {}",
                failures,
                last_error.as_deref().unwrap_or("unknown")
            )),
        )
    } else {
        (
            "partial",
            Some(format!(
                "{} of {} transfers failed. Last error: {}",
                failures,
                photos_synced + failures,
                last_error.as_deref().unwrap_or("unknown")
            )),
        )
    };

    update_sync_log(
        pool,
        log_id,
        status,
        photos_synced,
        bytes_synced,
        error_detail.as_deref(),
    )
    .await;

    let now = Utc::now().to_rfc3339();
    let db_status = if failures == 0 { "success" } else { "partial" };
    let db_error = error_detail.as_deref();

    if let Err(e) = sqlx::query(
        "UPDATE backup_servers SET last_sync_at = ?, last_sync_status = ?, \
         last_sync_error = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(db_status)
    .bind(db_error)
    .bind(&server.id)
    .execute(pool)
    .await
    {
        tracing::warn!(server_id = %server.id, error = %e, "Failed to update backup server sync status");
    }

    tracing::info!(
        "Backup sync to '{}' complete: {} synced, {} failed, {} bytes",
        server.name,
        photos_synced,
        failures,
        bytes_synced
    );
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Fetch the set of IDs the remote backup already has for a given list endpoint.
/// Returns an empty set on any error (graceful degradation to full sync).
async fn fetch_remote_ids(
    client: &reqwest::Client,
    base_url: &str,
    path: &str,
    api_key: &Option<String>,
) -> HashSet<String> {
    let mut req = client.get(format!("{}{}", base_url, path));
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            // Parse as array of objects with an "id" field
            #[derive(serde::Deserialize)]
            struct IdOnly {
                id: String,
            }
            match resp.json::<Vec<IdOnly>>().await {
                Ok(items) => items.into_iter().map(|i| i.id).collect(),
                Err(e) => {
                    tracing::warn!("Failed to parse remote ID list from {}: {}", path, e);
                    HashSet::new()
                }
            }
        }
        Ok(resp) => {
            tracing::warn!(
                "Remote {} returned HTTP {} — falling back to full sync",
                path,
                resp.status()
            );
            HashSet::new()
        }
        Err(e) => {
            tracing::warn!(
                "Failed to fetch remote IDs from {}: {} — falling back to full sync",
                path,
                e
            );
            HashSet::new()
        }
    }
}

/// Read a file from disk, compute its SHA-256 checksum, and POST it to the
/// backup server with integrity headers. Returns `Ok(())` on success or
/// `Err(description)` on failure.
async fn send_file(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    storage_root: &std::path::Path,
    item_id: &str,
    file_path: &str,
    source: &str,
) -> Result<(), String> {
    let full_path = storage_root.join(file_path);
    if !tokio::fs::try_exists(&full_path)
        .await
        .unwrap_or(false)
    {
        return Err("file not found on disk".to_string());
    }

    let file_data = tokio::fs::read(&full_path)
        .await
        .map_err(|e| format!("read error: {}", e))?;

    // Compute SHA-256 checksum for integrity verification
    let hash = Sha256::digest(&file_data);
    let hash_hex = hex::encode(hash);

    let mut req = client
        .post(format!("{}/backup/receive", base_url))
        .header("X-Photo-Id", item_id)
        .header("X-File-Path", file_path)
        .header("X-Source", source)
        .header("X-Content-Hash", hash_hex.as_str())
        .body(file_data);

    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => Ok(()),
        Ok(resp) => Err(format!("HTTP {}", resp.status())),
        Err(e) => Err(e.to_string()),
    }
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
    if let Err(e) = sqlx::query(
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
    .await
    {
        // Without this update the sync log row stays stuck in "running" state
        // permanently, so operators should be alerted.
        tracing::error!(log_id = log_id, error = %e, "Failed to update backup sync log — row stuck in 'running' state");
    }
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

            // Skip if another sync (manual trigger) is already running for this server
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

            let api_key: Option<String> = sqlx::query_scalar(
                "SELECT api_key FROM backup_servers WHERE id = ?",
            )
            .bind(&server.id)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();

            run_sync(&pool, &storage_root, server, &api_key, &log_id).await;
            drop(guard); // Release the concurrency lock
        }
    }
}
