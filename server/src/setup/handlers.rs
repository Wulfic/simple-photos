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
        version: "0.6.9".to_string(),
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
    let password_hash =
        bcrypt::hash(&req.password, state.config.auth.bcrypt_cost)
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
        &state.pool,
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
/// Discover Simple Photos servers on the local network via UDP broadcast.
/// Only works during first-run setup (zero users in DB) — no auth required.
pub async fn discover(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Guard: only works when no users exist
    let user_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await?;

    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed.".into(),
        ));
    }

    // UDP broadcast discovery — completes in ~6 seconds
    let broadcast_results = tokio::task::spawn_blocking(|| {
        crate::backup::broadcast::discover_via_broadcast(std::time::Duration::from_secs(6))
    })
    .await
    .unwrap_or_default();

    let servers: Vec<serde_json::Value> = broadcast_results
        .into_iter()
        .map(|b| serde_json::json!({
            "address": b.address,
            "name": b.name,
            "version": b.version,
        }))
        .collect();

    tracing::info!("Setup discovery: found {} servers via broadcast", servers.len());

    Ok(Json(serde_json::json!({ "servers": servers })))
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
    let password_hash =
        bcrypt::hash(&req.password, state.config.auth.bcrypt_cost)
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
        &state.pool,
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

