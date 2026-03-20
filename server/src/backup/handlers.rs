//! Backup server management endpoints.
//!
//! CRUD for registered backup destinations, LAN server discovery
//! (UDP broadcast + brute-force HTTP probe fallback), backup-mode
//! toggling (primary/backup with auto-generated API key), and the
//! audio-backup-enabled setting.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::setup::admin::require_admin;
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
    .fetch_all(&state.read_pool)
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

    // Validate and sanitize inputs
    let address = req.address.trim().to_string();
    if address.is_empty() {
        return Err(AppError::BadRequest("Address is required".into()));
    }
    let name = sanitize::sanitize_display_name(&req.name, 200)
        .map_err(|reason| AppError::BadRequest(reason.into()))?;

    // Check for duplicates
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM backup_servers WHERE address = ?",
    )
    .bind(&address)
    .fetch_one(&state.read_pool)
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
    .bind(&name)
    .bind(&address)
    .bind(&req.api_key)
    .bind(freq)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    tracing::info!("Added backup server '{}' at {}", name, address);

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "name": name,
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
    .fetch_one(&state.read_pool)
    .await?;

    if !exists {
        return Err(AppError::NotFound);
    }

    // Validate name outside the transaction (sanitize may reject input).
    let safe_name = req
        .name
        .as_ref()
        .map(|n| sanitize::sanitize_display_name(n, 200))
        .transpose()
        .map_err(|reason| AppError::BadRequest(reason.into()))?;

    // Transaction: apply all field updates atomically so a mid-request
    // failure can't leave the row in a half-updated state.
    let mut tx = state.pool.begin().await?;

    if let Some(ref name) = safe_name {
        sqlx::query("UPDATE backup_servers SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(ref address) = req.address {
        sqlx::query("UPDATE backup_servers SET address = ? WHERE id = ?")
            .bind(address.trim())
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(ref api_key) = req.api_key {
        sqlx::query("UPDATE backup_servers SET api_key = ? WHERE id = ?")
            .bind(api_key)
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(freq) = req.sync_frequency_hours {
        sqlx::query("UPDATE backup_servers SET sync_frequency_hours = ? WHERE id = ?")
            .bind(freq.max(1))
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(enabled) = req.enabled {
        sqlx::query("UPDATE backup_servers SET enabled = ? WHERE id = ?")
            .bind(enabled)
            .bind(&server_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

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
    .fetch_optional(&state.read_pool)
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
            api_key: None, // UDP broadcast doesn't carry the API key
        })
        .collect();

    let our_port = state.config.server.port;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    let mut existing_addrs: std::collections::HashSet<String> =
        discovered.iter().map(|d| d.address.clone()).collect();

    // ── Phase 1: Probe local/Docker hosts on likely backup ports ────────
    // Probes 127.0.0.1 (native co-located), host.docker.internal
    // (Docker-to-host), and 172.17.0.1 (Docker default gateway fallback)
    // so any combination of Docker + native servers is discoverable.
    let mut local_ports: Vec<u16> = Vec::new();
    // Ports adjacent to the server's own port (e.g. 8081-8089 when server is 8080)
    let base = (our_port / 10) * 10; // floor to nearest 10
    for p in base..=(base + 9) {
        if p != our_port {
            local_ports.push(p);
        }
    }
    // Also always check common server ports
    for &p in &[3000u16, 3001, 3002, 3003, 8080, 8081, 8082, 8083, 8443] {
        if p != our_port && !local_ports.contains(&p) {
            local_ports.push(p);
        }
    }

    // Hosts to probe: 127.0.0.1 (native), host.docker.internal (Docker-to-host),
    // and 172.17.0.1 (Docker default gateway fallback).
    let probe_hosts: Vec<&str> = vec!["127.0.0.1", "host.docker.internal", "172.17.0.1"];

    let mut local_futures = Vec::new();
    for &port in &local_ports {
        for &host in &probe_hosts {
            let addr = format!("{}:{}", host, port);
            if existing_addrs.contains(&addr) {
                continue;
            }
            let c = client.clone();
            let host_owned = host.to_string();
            // Try /api/discover/info first — it's a loopback-only endpoint that
            // returns the backup mode and API key, enabling zero-touch registration
            // of Docker containers and other co-located backup instances.
            local_futures.push(async move {
                let info_url = format!("http://{}:{}/api/discover/info", host_owned, port);
                let health_url = format!("http://{}:{}/health", host_owned, port);

                // ── Primary probe: /api/discover/info ─────────────────────────
                if let Ok(resp) = c.get(&info_url).send().await {
                    if resp.status().is_success() {
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
                                // Only present when the remote server is in backup mode
                                let api_key = body
                                    .get("api_key")
                                    .and_then(|k| k.as_str())
                                    .filter(|k| !k.is_empty())
                                    .map(|k| k.to_string());
                                return Some(DiscoveredServer {
                                    address: format!("{}:{}", host_owned, port),
                                    name,
                                    version,
                                    api_key,
                                });
                            }
                        }
                    }
                }
                // ── Fallback probe: /health (older servers) ───────────────────
                match c.get(&health_url).send().await {
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
                                return Some(DiscoveredServer {
                                    address: format!("{}:{}", host_owned, port),
                                    name,
                                    version,
                                    api_key: None,
                                });
                            }
                        }
                    }
                    _ => {}
                }
                None
            });
        }
    }

    let local_results = futures_util::future::join_all(local_futures).await;
    for result in local_results.into_iter().flatten() {
        existing_addrs.insert(result.address.clone());
        discovered.push(result);
    }

    // ── Phase 2: Derive actual subnet from server's base_url ─────────────
    // Parse the server's own IP from its configured base_url so we scan
    // the correct subnet instead of only hardcoded ones.
    let mut subnets: Vec<String> = Vec::new();
    if let Ok(url) = reqwest::Url::parse(&state.config.server.base_url) {
        if let Some(host) = url.host_str() {
            // Extract the first 3 octets for a /24 subnet scan
            let parts: Vec<&str> = host.split('.').collect();
            if parts.len() == 4 {
                let subnet = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
                subnets.push(subnet);
            }
        }
    }
    // Also try to get the local IP for subnet inference
    if let Some(local_ip) = broadcast::get_local_ip() {
        let parts: Vec<&str> = local_ip.split('.').collect();
        if parts.len() == 4 {
            let subnet = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
            if !subnets.contains(&subnet) { subnets.push(subnet); }
        }
    }
    // Resolve host.docker.internal to discover the Docker host's LAN subnet
    if let Ok(addrs) = tokio::net::lookup_host("host.docker.internal:0").await {
        for addr in addrs {
            let ip = addr.ip().to_string();
            let parts: Vec<&str> = ip.split('.').collect();
            if parts.len() == 4 {
                let subnet = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
                if !subnets.contains(&subnet) { subnets.push(subnet); }
            }
        }
    }
    // Add common fallback subnets that aren't already covered
    for s in &["192.168.1", "192.168.0", "10.0.0"] {
        if !subnets.iter().any(|existing| existing == s) {
            subnets.push(s.to_string());
        }
    }

    // ── Phase 3: LAN subnet scan (concurrent, with concurrency limit) ────
    let ports = [our_port, 8080, 8081, 8082, 8083, 3000];

    // Use a semaphore to limit concurrent connections and avoid flooding
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(50));

    let mut lan_futures = Vec::new();
    for subnet in &subnets {
        for host_id in 1..=254u8 {
            let ip = format!("{}.{}", subnet, host_id);
            for &port in &ports {
                let addr = format!("{}:{}", ip, port);
                if existing_addrs.contains(&addr) {
                    continue;
                }
                let url = format!("http://{}/health", addr);
                let c = client.clone();
                let permit = sem.clone();
                lan_futures.push(async move {
                    let _permit = permit.acquire().await.ok()?;
                    match c.get(&url).send().await {
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
                                    return Some(DiscoveredServer {
                                        address: addr,
                                        name,
                                        version,
                                        api_key: None, // LAN-discovered servers require manual API key entry
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                    None
                });
            }
        }
    }

    let lan_results = futures_util::future::join_all(lan_futures).await;
    for result in lan_results.into_iter().flatten() {
        discovered.push(result);
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
    .fetch_optional(&state.read_pool)
    .await?
    .unwrap_or_else(|| "primary".to_string());

    let local_ip = broadcast::get_local_ip().unwrap_or_else(|| "unknown".to_string());
    let port = state.config.server.port;

    // Include the API key when in backup mode
    let api_key: Option<String> = if mode == "backup" {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
        )
        .fetch_optional(&state.read_pool)
        .await?
        .flatten()
    } else {
        None
    };

    Ok(Json(BackupModeResponse {
        mode,
        server_ip: local_ip.clone(),
        server_address: format!("{}:{}", local_ip, port),
        port,
        api_key,
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

    // Include the API key when in backup mode
    let api_key_val: Option<String> = if mode == "backup" {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
        )
        .fetch_optional(&state.read_pool)
        .await?
        .flatten()
    } else {
        None
    };

    Ok(Json(BackupModeResponse {
        mode,
        server_ip: local_ip.clone(),
        server_address: format!("{}:{}", local_ip, port),
        port,
        api_key: api_key_val,
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
    let enabled: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
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
