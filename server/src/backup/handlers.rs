use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::is_media_file;
use crate::media::mime_from_extension;
use crate::state::AppState;

use super::broadcast;
use super::models::*;

// ── Backup Server Management ─────────────────────────────────────────────────

/// GET /api/admin/backup/servers
/// List all configured backup servers (admin only).
pub async fn list_backup_servers(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<BackupServerListResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let servers = sqlx::query_as::<_, BackupServer>(
        "SELECT id, name, address, sync_frequency_hours, last_sync_at, \
         last_sync_status, last_sync_error, enabled, created_at \
         FROM backup_servers ORDER BY created_at ASC",
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(BackupServerListResponse { servers }))
}

/// POST /api/admin/backup/servers
/// Add a new backup server (admin only).
pub async fn add_backup_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<AddBackupServerRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    require_admin(&state, &auth).await?;

    // Validate address format
    let address = req.address.trim().to_string();
    if address.is_empty() {
        return Err(AppError::BadRequest("Address is required".into()));
    }

    // Check for duplicates
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM backup_servers WHERE address = ?",
    )
    .bind(&address)
    .fetch_one(&state.pool)
    .await?;

    if exists {
        return Err(AppError::Conflict(format!(
            "Backup server at {} already exists",
            address
        )));
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let freq = req.sync_frequency_hours.unwrap_or(24).max(1);

    sqlx::query(
        "INSERT INTO backup_servers (id, name, address, api_key, sync_frequency_hours, \
         last_sync_status, enabled, created_at) \
         VALUES (?, ?, ?, ?, ?, 'never', 1, ?)",
    )
    .bind(&id)
    .bind(&req.name)
    .bind(&address)
    .bind(&req.api_key)
    .bind(freq)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    tracing::info!("Added backup server '{}' at {}", req.name, address);

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "name": req.name,
            "address": address,
            "sync_frequency_hours": freq,
        })),
    ))
}

/// PUT /api/admin/backup/servers/:id
/// Update a backup server's configuration (admin only).
pub async fn update_backup_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
    Json(req): Json<UpdateBackupServerRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    // Verify server exists
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM backup_servers WHERE id = ?",
    )
    .bind(&server_id)
    .fetch_one(&state.pool)
    .await?;

    if !exists {
        return Err(AppError::NotFound);
    }

    if let Some(ref name) = req.name {
        sqlx::query("UPDATE backup_servers SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&server_id)
            .execute(&state.pool)
            .await?;
    }
    if let Some(ref address) = req.address {
        sqlx::query("UPDATE backup_servers SET address = ? WHERE id = ?")
            .bind(address.trim())
            .bind(&server_id)
            .execute(&state.pool)
            .await?;
    }
    if let Some(ref api_key) = req.api_key {
        sqlx::query("UPDATE backup_servers SET api_key = ? WHERE id = ?")
            .bind(api_key)
            .bind(&server_id)
            .execute(&state.pool)
            .await?;
    }
    if let Some(freq) = req.sync_frequency_hours {
        sqlx::query("UPDATE backup_servers SET sync_frequency_hours = ? WHERE id = ?")
            .bind(freq.max(1))
            .bind(&server_id)
            .execute(&state.pool)
            .await?;
    }
    if let Some(enabled) = req.enabled {
        sqlx::query("UPDATE backup_servers SET enabled = ? WHERE id = ?")
            .bind(enabled)
            .bind(&server_id)
            .execute(&state.pool)
            .await?;
    }

    Ok(Json(serde_json::json!({
        "message": "Backup server updated",
        "id": server_id,
    })))
}

/// DELETE /api/admin/backup/servers/:id
/// Remove a backup server (admin only).
pub async fn remove_backup_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<StatusCode, AppError> {
    require_admin(&state, &auth).await?;

    let result = sqlx::query("DELETE FROM backup_servers WHERE id = ?")
        .bind(&server_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    tracing::info!("Removed backup server {}", server_id);

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/admin/backup/servers/:id/status
/// Check if a backup server is reachable and get its version.
pub async fn check_backup_server_status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<Json<BackupServerStatus>, AppError> {
    require_admin(&state, &auth).await?;

    let address: String = sqlx::query_scalar(
        "SELECT address FROM backup_servers WHERE id = ?",
    )
    .bind(&server_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    // Try to reach the server's health endpoint
    let url = format!("http://{}/health", address);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    match client.get(&url).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let version = body
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                Ok(Json(BackupServerStatus {
                    reachable: true,
                    version: Some(version),
                    error: None,
                }))
            } else {
                Ok(Json(BackupServerStatus {
                    reachable: false,
                    version: None,
                    error: Some(format!("Server responded with status {}", resp.status())),
                }))
            }
        }
        Err(e) => Ok(Json(BackupServerStatus {
            reachable: false,
            version: None,
            error: Some(format!("Connection failed: {}", e)),
        })),
    }
}

/// GET /api/admin/backup/discover
/// Discover Simple Photos servers on the local network via UDP broadcast.
/// Backup-mode servers broadcast their presence and respond to discovery probes.
pub async fn discover_servers(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<DiscoverResponse>, AppError> {
    require_admin(&state, &auth).await?;

    // Use UDP broadcast discovery — much faster than sequential HTTP probing.
    // Backup-mode servers broadcast beacons every 5 seconds and respond to probes.
    let broadcast_results = tokio::task::spawn_blocking(|| {
        broadcast::discover_via_broadcast(std::time::Duration::from_secs(6))
    })
    .await
    .unwrap_or_default();

    let mut discovered: Vec<DiscoveredServer> = broadcast_results
        .into_iter()
        .map(|b| DiscoveredServer {
            address: b.address,
            name: b.name,
            version: b.version,
        })
        .collect();

    // Also do a quick HTTP probe on common addresses as fallback
    let our_port = state.config.server.port;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    let subnets = ["192.168.1", "192.168.0", "10.0.0"];
    let ports = [our_port, 8080, 3000];
    let existing_addrs: std::collections::HashSet<String> =
        discovered.iter().map(|d| d.address.clone()).collect();

    for subnet in &subnets {
        for host_id in 1..=254u8 {
            let ip = format!("{}.{}", subnet, host_id);
            for &port in &ports {
                let addr = format!("{}:{}", ip, port);
                if existing_addrs.contains(&addr) {
                    continue;
                }
                let url = format!("http://{}/health", addr);
                let client = client.clone();
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            if body.get("service").and_then(|s| s.as_str())
                                == Some("simple-photos")
                            {
                                let name = body
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("Unknown")
                                    .to_string();
                                let version = body
                                    .get("version")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                discovered.push(DiscoveredServer {
                                    address: addr,
                                    name,
                                    version,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(Json(DiscoverResponse {
        servers: discovered,
    }))
}

// ── Backup Mode Endpoints ────────────────────────────────────────────────────

/// GET /api/admin/backup/mode
/// Returns the current server mode ("primary" or "backup") and the server's local IP.
pub async fn get_backup_mode(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<BackupModeResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'backup_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "primary".to_string());

    let local_ip = broadcast::get_local_ip().unwrap_or_else(|| "unknown".to_string());
    let port = state.config.server.port;

    Ok(Json(BackupModeResponse {
        mode,
        server_ip: local_ip.clone(),
        server_address: format!("{}:{}", local_ip, port),
        port,
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
        _ => return Err(AppError::BadRequest("Mode must be 'primary' or 'backup'".into())),
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

    Ok(Json(BackupModeResponse {
        mode,
        server_ip: local_ip.clone(),
        server_address: format!("{}:{}", local_ip, port),
        port,
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

    // 1. Sync photos from the photos table
    let photos: Vec<(String, String, i64)> = match sqlx::query_as(
        "SELECT id, file_path, size_bytes FROM photos",
    )
    .fetch_all(pool)
    .await
    {
        Ok(p) => p,
        Err(e) => {
            update_sync_log(pool, log_id, "error", 0, 0, Some(&e.to_string())).await;
            return;
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

// ── Auto-Scan Background Task ────────────────────────────────────────────────

/// Background task: automatically scan the storage directory for new files
/// every 24 hours (or when triggered by an API call).
pub async fn background_auto_scan_task(
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
) {
    // Wait 30 seconds after startup before first check
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600)); // Check every hour

    loop {
        interval.tick().await;

        // Check when we last scanned
        let last_scan: String = sqlx::query_scalar(
            "SELECT value FROM server_settings WHERE key = 'last_auto_scan'",
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

        let should_scan = if last_scan.is_empty() {
            true
        } else if let Ok(last_dt) = chrono::DateTime::parse_from_rfc3339(&last_scan) {
            let elapsed = Utc::now() - last_dt.with_timezone(&Utc);
            elapsed.num_hours() >= 24
        } else {
            true
        };

        if !should_scan {
            continue;
        }

        tracing::info!("Starting automatic storage scan...");
        let count = run_auto_scan(&pool, &storage_root).await;
        if count > 0 {
            tracing::info!("Auto-scan complete: registered {} new files", count);
        } else {
            tracing::info!("Auto-scan complete: no new files found");
        }

        // Update last scan time
        let now = Utc::now().to_rfc3339();
        let _ = sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES ('last_auto_scan', ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(&now)
        .execute(&pool)
        .await;
    }
}

/// POST /api/admin/photos/auto-scan
/// Trigger an immediate auto-scan (called when web UI or app opens).
/// This is non-blocking — it kicks off a background scan and returns immediately.
pub async fn trigger_auto_scan(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state.pool.clone();
    let storage_root = state.storage_root.read().await.clone();

    // Spawn as a background task so the UI doesn't block
    tokio::spawn(async move {
        let count = run_auto_scan(&pool, &storage_root).await;
        if count > 0 {
            tracing::info!("On-demand scan: registered {} new files", count);
        }

        // Update last scan time
        let now = Utc::now().to_rfc3339();
        let _ = sqlx::query(
            "INSERT INTO server_settings (key, value) VALUES ('last_auto_scan', ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(&now)
        .execute(&pool)
        .await;
    });

    Ok(Json(serde_json::json!({
        "message": "Scan started in background",
    })))
}

/// Scan storage directory and register any unregistered media files for ALL users.
async fn run_auto_scan(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
) -> i64 {
    // Get the first admin user to assign new photos to
    let admin_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let admin_id = match admin_id {
        Some(id) => id,
        None => return 0, // No admin user yet
    };

    // Get already-registered file paths
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT file_path FROM photos",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let existing_set: std::collections::HashSet<String> = existing.into_iter().collect();

    let mut new_count = 0i64;
    let mut queue = vec![storage_root.to_path_buf()];

    while let Some(dir) = queue.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() && is_media_file(&name) {
                    let abs_path = entry.path();
                    // Normalize to forward slashes so DB paths are consistent across OS
                    let rel_path = abs_path
                        .strip_prefix(storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    if existing_set.contains(&rel_path) {
                        continue;
                    }

                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            dt.to_rfc3339()
                        })
                    });

                    let mime = mime_from_extension(&name).to_string();
                    let media_type = if mime.starts_with("video/") {
                        "video"
                    } else if mime == "image/gif" {
                        "gif"
                    } else {
                        "photo"
                    };

                    let photo_id = Uuid::new_v4().to_string();
                    let now = Utc::now().to_rfc3339();
                    let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

                    let _ = sqlx::query(
                        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                         size_bytes, width, height, taken_at, thumb_path, created_at) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?)",
                    )
                    .bind(&photo_id)
                    .bind(&admin_id)
                    .bind(&name)
                    .bind(&rel_path)
                    .bind(&mime)
                    .bind(media_type)
                    .bind(size)
                    .bind(&modified)
                    .bind(&thumb_rel)
                    .bind(&now)
                    .execute(pool)
                    .await;

                    new_count += 1;
                }
            }
        }
    }

    new_count
}

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn require_admin(state: &AppState, auth: &AuthUser) -> Result<(), AppError> {
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    Ok(())
}
