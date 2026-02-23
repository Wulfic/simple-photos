use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

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
/// Attempt to discover Simple Photos servers on the local network.
/// Uses a simple HTTP probe on common ports across the local subnet.
pub async fn discover_servers(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<DiscoverResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let mut discovered = Vec::new();

    // Get this server's port to try on other hosts
    let our_port = state.config.server.port;

    // Try common local network addresses
    // In a real implementation this would use mDNS/UDP broadcast,
    // but for simplicity we probe common subnet ranges.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    // Probe the local subnet (192.168.1.x and 192.168.0.x are most common)
    let subnets = ["192.168.1", "192.168.0", "10.0.0"];
    let ports = [our_port, 8080, 3000];

    for subnet in &subnets {
        for host_id in 1..=254u8 {
            let ip = format!("{}.{}", subnet, host_id);

            for &port in &ports {
                let addr = format!("{}:{}", ip, port);
                let url = format!("http://{}/health", addr);
                let client = client.clone();

                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            // Check if it's a Simple Photos server
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
