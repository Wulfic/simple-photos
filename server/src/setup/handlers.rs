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
    if req.username.len() < 3 || req.username.len() > 50 {
        return Err(AppError::BadRequest(
            "Username must be between 3 and 50 characters".into(),
        ));
    }
    if !req.username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::BadRequest(
            "Username may only contain letters, numbers, and underscores".into(),
        ));
    }

    // ── Validate password ───────────────────────────────────────────────────
    if req.password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".into(),
        ));
    }
    if req.password.len() > 128 {
        return Err(AppError::BadRequest(
            "Password must not exceed 128 characters".into(),
        ));
    }
    let has_upper = req.password.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = req.password.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = req.password.chars().any(|c| c.is_ascii_digit());
    if !has_upper || !has_lower || !has_digit {
        return Err(AppError::BadRequest(
            "Password must contain at least one uppercase letter, one lowercase letter, and one digit".into(),
        ));
    }

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

