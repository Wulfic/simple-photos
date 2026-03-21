//! First-run setup wizard API endpoints.
//!
//! These endpoints are used by the web frontend's setup wizard to bootstrap
//! the application on first run. They allow creating the initial admin user
//! without requiring authentication (since no users exist yet).
//!
//! Security: `POST /api/setup/init` only works when zero users exist in the DB.
//! Once the first user is created, these endpoints become effectively read-only.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::tokens::issue_tokens;
use crate::auth::validation;
use crate::error::AppError;
use crate::state::AppState;

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SetupStatusResponse {
    /// Whether initial setup has been completed (at least one user exists)
    pub setup_complete: bool,
    /// Whether new user registration is currently enabled
    pub registration_open: bool,
    /// Server version
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct InitSetupRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct InitSetupResponse {
    pub user_id: String,
    pub username: String,
    pub message: String,
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// Check if initial setup has been completed.
///
/// This endpoint is public (no auth required) so the web frontend can
/// determine whether to show the setup wizard or the login page.
///
/// Returns:
/// - `setup_complete: false` → Show first-run wizard
/// - `setup_complete: true` → Show normal login
pub async fn status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatusResponse>, AppError> {
    let user_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await?;

    Ok(Json(SetupStatusResponse {
        setup_complete: user_count > 0,
        registration_open: state.config.auth.allow_registration,
        version: crate::VERSION.to_string(),
    }))
}

/// Create the first user during initial setup.
///
/// # Security
/// This endpoint ONLY works when the database has zero users.
/// Once any user exists, this returns 403 Forbidden.
///
/// The first user is created with the same validation rules as normal
/// registration (password complexity, username format, etc.).
pub async fn init(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<InitSetupRequest>,
) -> Result<(StatusCode, Json<InitSetupResponse>), AppError> {
    // ── Guard: only works when no users exist ────────────────────────────────
    let user_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await?;

    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed. Use the normal registration endpoint.".into(),
        ));
    }

    // ── Validate username ───────────────────────────────────────────────────
    validation::validate_username(&req.username)?;

    // ── Validate password ───────────────────────────────────────────────────
    validation::validate_password(&req.password)?;

    // ── Create user ─────────────────────────────────────────────────────────
    let user_id = Uuid::new_v4().to_string();
    let password_clone = req.password.clone();
    let cost = state.config.auth.bcrypt_cost;
    let password_hash = tokio::task::spawn_blocking(move || bcrypt::hash(&password_clone, cost))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, role) VALUES (?, ?, ?, ?, ?, 'admin')",
    )
    .bind(&user_id)
    .bind(&req.username)
    .bind(&password_hash)
    .bind(&now)
    .bind(state.config.storage.default_quota_bytes as i64)
    .execute(&state.pool)
    .await?;

    audit::log(
        &state,
        AuditEvent::Register,
        Some(&user_id),
        &headers,
        Some(serde_json::json!({
            "username": req.username,
            "method": "first_run_setup"
        })),
    )
    .await;

    tracing::info!(
        "First-run setup complete: user '{}' created ({})",
        req.username,
        user_id
    );

    Ok((
        StatusCode::CREATED,
        Json(InitSetupResponse {
            user_id,
            username: req.username,
            message: "Setup complete! You can now log in.".into(),
        }),
    ))
}

// ── Backup Pairing ──────────────────────────────────────────────────────────

// ── Server Discovery (during first-run setup) ───────────────────────────────

/// GET /api/setup/discover
///
/// Discover Simple Photos servers on the local network.
///
/// Uses the dedicated discovery port (default 3301) to find servers with a
/// single probe per IP, supplemented by UDP broadcast and localhost fallbacks.
///
/// Only works during first-run setup (zero users in DB) — no auth required.
pub async fn discover(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Guard: only works when no users exist
    let user_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&state.read_pool)
            .await?;

    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed.".into(),
        ));
    }

    let our_port = state.config.server.port;
    let discovery_port = state.config.server.discovery_port;
    let mut discovered: Vec<serde_json::Value> = Vec::new();
    let mut existing_addrs = std::collections::HashSet::new();

    // ── Phase 1: UDP broadcast discovery (~3 seconds) ────────────────────
    let broadcast_results = tokio::task::spawn_blocking(|| {
        crate::backup::broadcast::discover_via_broadcast(std::time::Duration::from_secs(3))
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Broadcast discovery task panicked: {}", e);
        Vec::new()
    });

    for b in broadcast_results {
        existing_addrs.insert(b.address.clone());
        discovered.push(serde_json::json!({
            "address": b.address,
            "name": b.name,
            "version": b.version,
        }));
    }

    // ── Phase 2: Localhost + Docker-host probing ─────────────────────────
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    let mut probe_hosts: Vec<String> = vec![
        "127.0.0.1".to_string(),
        "host.docker.internal".to_string(),
        "172.17.0.1".to_string(),
    ];
    if let Some(gw) = crate::backup::broadcast::get_default_gateway() {
        if !probe_hosts.contains(&gw) {
            probe_hosts.push(gw);
        }
    }

    // Build flat list of local probes: (host, port, is_discovery)
    let mut local_probes: Vec<(String, u16, bool)> = Vec::new();

    // Probe discovery port on local hosts first
    if discovery_port != 0 {
        for host in &probe_hosts {
            local_probes.push((host.clone(), discovery_port, true));
        }
    }

    // Fallback: probe common ports on local hosts via /api/discover/info + /health
    let mut local_ports: Vec<u16> = Vec::new();
    let base = (our_port / 10) * 10;
    for p in base..=(base + 9) {
        if p != our_port && p != discovery_port { local_ports.push(p); }
    }
    for &p in &[3000u16, 3001, 3002, 3003, 8080, 8081, 8082, 8083, 8443] {
        if p != our_port && p != discovery_port && !local_ports.contains(&p) { local_ports.push(p); }
    }

    for &port in &local_ports {
        for host in &probe_hosts {
            let addr = format!("{}:{}", host, port);
            if existing_addrs.contains(&addr) { continue; }
            local_probes.push((host.clone(), port, false));
        }
    }

    let mut local_futures = Vec::new();
    for (host_owned, port, is_discovery) in local_probes {
        let c = client.clone();
        local_futures.push(async move {
            if is_discovery {
                let url = format!("http://{}:{}/", host_owned, port);
                if let Ok(resp) = c.get(&url).send().await {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            if body.get("service").and_then(|s| s.as_str()) == Some("simple-photos") {
                                let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string();
                                let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                                let actual_port = body.get("port").and_then(|p| p.as_u64()).map(|p| p as u16).unwrap_or(port);
                                return Some(serde_json::json!({
                                    "address": format!("{}:{}", host_owned, actual_port),
                                    "name": name,
                                    "version": version,
                                }));
                            }
                        }
                    }
                }
            } else {
                let info_url = format!("http://{}:{}/api/discover/info", host_owned, port);
                let health_url = format!("http://{}:{}/health", host_owned, port);

                if let Ok(resp) = c.get(&info_url).send().await {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            if body.get("service").and_then(|s| s.as_str()) == Some("simple-photos") {
                                let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string();
                                let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                                return Some(serde_json::json!({
                                    "address": format!("{}:{}", host_owned, port),
                                    "name": name,
                                    "version": version,
                                }));
                            }
                        }
                    }
                }
                if let Ok(resp) = c.get(&health_url).send().await {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            if body.get("service").and_then(|s| s.as_str()) == Some("simple-photos") {
                                let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string();
                                let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                                return Some(serde_json::json!({
                                    "address": format!("{}:{}", host_owned, port),
                                    "name": name,
                                    "version": version,
                                }));
                            }
                        }
                    }
                }
            }
            None
        });
    }

    let local_results = futures_util::future::join_all(local_futures).await;
    for result in local_results.into_iter().flatten() {
        let addr = result.get("address").and_then(|a| a.as_str()).unwrap_or("").to_string();
        if !existing_addrs.contains(&addr) {
            existing_addrs.insert(addr);
            discovered.push(result);
        }
    }

    tracing::info!(
        "Setup discovery Phase 2 complete: {} servers found via local probes (discovery_port={})",
        discovered.len(),
        discovery_port
    );

    // ── Phase 3: LAN subnet scan via discovery port ──────────────────────
    let mut subnets: Vec<String> = Vec::new();
    if let Ok(url) = reqwest::Url::parse(&state.config.server.base_url) {
        if let Some(host) = url.host_str() {
            let parts: Vec<&str> = host.split('.').collect();
            if parts.len() == 4 {
                subnets.push(format!("{}.{}.{}", parts[0], parts[1], parts[2]));
            }
        }
    }
    if let Some(local_ip) = crate::backup::broadcast::get_local_ip() {
        tracing::debug!("Setup discovery: local IP = {}", local_ip);
        let parts: Vec<&str> = local_ip.split('.').collect();
        if parts.len() == 4 {
            let subnet = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
            if !subnets.contains(&subnet) { subnets.push(subnet); }
        }
    }
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

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(100));

    // Build a flat list of (ip, port, use_discovery_protocol) probes
    let mut probes: Vec<(String, u16, bool)> = Vec::new();
    for subnet in &subnets {
        for host_part in 1..=254u8 {
            let ip = format!("{}.{}", subnet, host_part);
            if discovery_port != 0 {
                let addr = format!("{}:{}", ip, discovery_port);
                if !existing_addrs.contains(&addr) {
                    probes.push((ip, discovery_port, true));
                }
            } else {
                for &port in &[our_port, 8080u16, 8081, 8082, 8083, 3000] {
                    let addr = format!("{}:{}", ip, port);
                    if !existing_addrs.contains(&addr) {
                        probes.push((ip.clone(), port, false));
                    }
                }
            }
        }
    }

    tracing::info!(
        "Setup discovery Phase 3: scanning {} subnets ({:?}) on port {}, {} total probes",
        subnets.len(),
        subnets,
        if discovery_port != 0 { discovery_port } else { our_port },
        probes.len()
    );

    let mut lan_futures = Vec::new();
    for (ip, port, is_discovery) in probes {
        let c = client.clone();
        let sem = sem.clone();
        lan_futures.push(async move {
            let _permit = sem.acquire().await;
            if is_discovery {
                // Probe the dedicated discovery port (default 3301)
                let url = format!("http://{}:{}/", ip, port);
                if let Ok(resp) = c.get(&url).send().await {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            if body.get("service").and_then(|s| s.as_str()) == Some("simple-photos") {
                                let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string();
                                let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                                let actual_port = body.get("port").and_then(|p| p.as_u64()).map(|p| p as u16).unwrap_or(port);
                                return Some(serde_json::json!({
                                    "address": format!("{}:{}", ip, actual_port),
                                    "name": name,
                                    "version": version,
                                }));
                            }
                        }
                    }
                }
            } else {
                // Fallback: probe /health on a specific server port
                let health_url = format!("http://{}:{}/health", ip, port);
                if let Ok(resp) = c.get(&health_url).send().await {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            if body.get("service").and_then(|s| s.as_str()) == Some("simple-photos") {
                                let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string();
                                let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                                return Some(serde_json::json!({
                                    "address": format!("{}:{}", ip, port),
                                    "name": name,
                                    "version": version,
                                }));
                            }
                        }
                    }
                }
            }
            None
        });
    }

    // Use FuturesUnordered + streaming so we collect results as they
    // complete. Previously `join_all` + `unwrap_or_default()` discarded
    // ALL results when the timeout fired — even those already finished.
    let mut stream: futures_util::stream::FuturesUnordered<_> =
        lan_futures.into_iter().collect();

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        match tokio::time::timeout_at(deadline, futures_util::StreamExt::next(&mut stream)).await {
            Ok(Some(Some(result))) => {
                let addr = result.get("address").and_then(|a| a.as_str()).unwrap_or("").to_string();
                if !existing_addrs.contains(&addr) {
                    existing_addrs.insert(addr);
                    discovered.push(result);
                }
            }
            Ok(Some(None)) => { /* probe returned None (no server found) */ }
            Ok(None) => break,   // stream exhausted — all probes done
            Err(_) => {
                tracing::warn!(
                    "Setup discovery: LAN scan timed out after 12s with {} servers found so far",
                    discovered.len()
                );
                break; // deadline reached — keep whatever we collected
            }
        }
    }

    tracing::info!("Setup discovery: found {} servers", discovered.len());

    Ok(Json(serde_json::json!({ "servers": discovered })))
}

#[derive(Debug, Deserialize)]
pub struct PairRequest {
    /// The address of the primary server (e.g. "192.168.1.10:8080")
    pub main_server_url: String,
    /// Admin username on the primary server
    pub username: String,
    /// Admin password on the primary server
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct PairResponse {
    pub message: String,
    pub user_id: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
    pub main_server_url: String,
}

/// POST /api/setup/pair
///
/// Pair this server as a backup of an existing primary Simple Photos instance.
///
/// # Security
/// Only works during first-run setup (zero users in DB).
///
/// # Flow
/// 1. Authenticates against the primary server with the given admin credentials
/// 2. Creates a local admin account with the same credentials
/// 3. Sets this server to "backup" mode
/// 4. Returns local auth tokens so the frontend is logged in immediately
pub async fn pair(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PairRequest>,
) -> Result<(StatusCode, Json<PairResponse>), AppError> {
    // ── Guard: only works when no users exist ────────────────────────────
    let user_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await?;

    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed.".into(),
        ));
    }

    // ── Normalise the main server URL ────────────────────────────────────
    let base = req.main_server_url.trim().trim_end_matches('/');
    let base_url = if base.starts_with("http://") || base.starts_with("https://") {
        base.to_string()
    } else {
        format!("http://{}", base)
    };

    // ── Authenticate against the primary server ──────────────────────────
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .danger_accept_invalid_certs(true)      // self-signed certs OK during setup
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    let login_url = format!("{}/api/auth/login", base_url);
    let login_body = serde_json::json!({
        "username": req.username,
        "password": req.password,
    });

    let resp = client
        .post(&login_url)
        .header("Content-Type", "application/json")
        .json(&login_body)
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!(
            "Cannot reach the primary server at {}: {}",
            base_url, e
        )))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::BadRequest(format!(
            "Primary server rejected the credentials (HTTP {}): {}",
            status, body
        )));
    }

    // We don't need the remote tokens — we only confirmed the credentials are valid.
    tracing::info!(
        "Successfully authenticated against primary server at {}",
        base_url
    );

    // ── Create local admin with the same credentials ─────────────────────
    let user_id = Uuid::new_v4().to_string();
    let password_clone = req.password.clone();
    let cost = state.config.auth.bcrypt_cost;
    let password_hash = tokio::task::spawn_blocking(move || bcrypt::hash(&password_clone, cost))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, role) \
         VALUES (?, ?, ?, ?, ?, 'admin')",
    )
    .bind(&user_id)
    .bind(&req.username)
    .bind(&password_hash)
    .bind(&now)
    .bind(state.config.storage.default_quota_bytes as i64)
    .execute(&state.pool)
    .await?;

    // ── Set backup mode ──────────────────────────────────────────────────
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('backup_mode', 'backup') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(&state.pool)
    .await?;

    // Store the primary server URL for future sync operations
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('primary_server_url', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&base_url)
    .execute(&state.pool)
    .await?;

    // Auto-generate a backup API key
    let api_key = Uuid::new_v4().to_string().replace('-', "");
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('backup_api_key', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&api_key)
    .execute(&state.pool)
    .await?;

    // ── Generate local auth tokens ───────────────────────────────────────
    let (access_token, refresh_token) = issue_tokens(&state, &user_id).await?;

    audit::log(
        &state,
        AuditEvent::Register,
        Some(&user_id),
        &headers,
        Some(serde_json::json!({
            "username": req.username,
            "method": "backup_pairing",
            "primary_server": base_url,
        })),
    )
    .await;

    tracing::info!(
        "Backup pairing complete: local admin '{}' created, paired with {}",
        req.username,
        base_url
    );

    Ok((
        StatusCode::CREATED,
        Json(PairResponse {
            message: "Paired successfully! This server is now a backup.".into(),
            user_id,
            username: req.username,
            access_token,
            refresh_token,
            main_server_url: base_url,
        }),
    ))
}

// ── Restore from Backup during Primary Setup ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct VerifyBackupRequest {
    /// The address of the backup server (e.g. "192.168.1.20:8080")
    pub address: String,
    /// Admin username on the backup server
    pub username: String,
    /// Admin password on the backup server
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyBackupResponse {
    pub address: String,
    pub name: String,
    pub version: String,
    pub api_key: Option<String>,
    pub photo_count: i64,
}

/// POST /api/setup/verify-backup
///
/// Verify connectivity to a backup server during primary setup with "restore" mode.
/// Authenticates against the backup server, retrieves its API key and photo count.
///
/// # Security
/// Only works during first-run setup (zero users in DB).
pub async fn verify_backup(
    State(state): State<AppState>,
    Json(req): Json<VerifyBackupRequest>,
) -> Result<Json<VerifyBackupResponse>, AppError> {
    // ── Guard: only works when no users exist ────────────────────────────
    let user_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await?;

    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed.".into(),
        ));
    }

    // ── Normalise the backup server URL ──────────────────────────────────
    let base = req.address.trim().trim_end_matches('/');
    let base_url = if base.starts_with("http://") || base.starts_with("https://") {
        base.to_string()
    } else {
        format!("http://{}", base)
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    // ── Authenticate against the backup server ───────────────────────────
    let login_url = format!("{}/api/auth/login", base_url);
    let login_body = serde_json::json!({
        "username": req.username,
        "password": req.password,
    });

    let resp = client
        .post(&login_url)
        .header("Content-Type", "application/json")
        .json(&login_body)
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!(
            "Cannot reach the backup server at {}: {}", base_url, e
        )))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::BadRequest(format!(
            "Backup server rejected the credentials (HTTP {}): {}", status, body
        )));
    }

    let login_data: serde_json::Value = resp.json().await
        .map_err(|e| AppError::Internal(format!("Failed to parse login response: {}", e)))?;

    let access_token = login_data.get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("No access_token in backup server login response".into()))?;

    // ── Get backup mode info (including API key) from the backup server ──
    let mode_url = format!("{}/api/admin/backup/mode", base_url);
    let mode_resp = client
        .get(&mode_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to get backup mode: {}", e)))?;

    let mut api_key: Option<String> = None;
    let mut server_name = "Backup Server".to_string();
    let mut version = "unknown".to_string();

    if mode_resp.status().is_success() {
        let mode_data: serde_json::Value = mode_resp.json().await.unwrap_or_default();
        api_key = mode_data.get("api_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }

    // ── Get server info from health endpoint ─────────────────────────────
    let health_url = format!("{}/health", base_url);
    if let Ok(health_resp) = client.get(&health_url).send().await {
        if let Ok(health_data) = health_resp.json::<serde_json::Value>().await {
            if let Some(name) = health_data.get("name").and_then(|v| v.as_str()) {
                server_name = name.to_string();
            }
            if let Some(ver) = health_data.get("version").and_then(|v| v.as_str()) {
                version = ver.to_string();
            }
        }
    }

    // ── Get photo count from the backup server ───────────────────────────
    let mut photo_count: i64 = 0;
    if let Some(ref key) = api_key {
        let list_url = format!("{}/api/backup/list", base_url);
        let list_resp = client
            .get(&list_url)
            .header("X-API-Key", key.as_str())
            .send()
            .await;

        if let Ok(resp) = list_resp {
            if resp.status().is_success() {
                if let Ok(photos) = resp.json::<Vec<serde_json::Value>>().await {
                    photo_count = photos.len() as i64;
                }
            }
        }
    }

    tracing::info!(
        "Verified backup server at {}: {} photos available for restore",
        base_url, photo_count
    );

    Ok(Json(VerifyBackupResponse {
        address: req.address.trim().to_string(),
        name: server_name,
        version,
        api_key,
        photo_count,
    }))
}

