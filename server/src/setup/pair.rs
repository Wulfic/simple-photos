//! Backup pairing and restore-verification endpoints.
//!
//! These endpoints handle pairing this server as a backup of an existing
//! primary Simple Photos instance, and verifying backup servers during
//! primary setup with "restore" mode.
//!
//! Helper functions (auth, registration, address detection) live in
//! [`super::pair_helpers`].

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::tokens::issue_tokens;
use crate::error::AppError;
use crate::state::AppState;

use super::pair_helpers::{
    authenticate_with_primary, configure_backup_mode, create_local_admin,
    determine_backup_address, normalize_server_url, register_backup_on_primary,
    trigger_initial_sync, verify_primary_is_not_backup, PrimaryAuthOutcome,
};

// ── Backup Pairing ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PairRequest {
    /// The address of the primary server (e.g. "192.168.1.10:8080")
    pub main_server_url: String,
    /// Admin username on the primary server
    pub username: String,
    /// Admin password on the primary server
    pub password: String,
    /// Optional 2FA code — required when the primary admin has TOTP enabled.
    /// On the first attempt (without this field) the server responds with
    /// `{ "requires_totp": true }` so the client can prompt for the code.
    pub totp_code: Option<String>,
}

// ── Backup Pairing Handler ──────────────────────────────────────────────────

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
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Guard: only works when no users exist
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await?;
    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed.".into(),
        ));
    }

    let base_url = normalize_server_url(&req.main_server_url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    // Authenticate against the primary (with optional 2FA)
    let remote_token = match authenticate_with_primary(
        &client,
        &base_url,
        &req.username,
        &req.password,
        req.totp_code.as_deref(),
    )
    .await?
    {
        PrimaryAuthOutcome::NeedsTotp => {
            return Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "requires_totp": true,
                    "message": "Primary server requires a 2FA code to complete pairing."
                })),
            ));
        }
        PrimaryAuthOutcome::Authenticated(token) => token,
    };

    tracing::info!(
        "Successfully authenticated against primary server at {}",
        base_url
    );

    verify_primary_is_not_backup(&client, &base_url, &remote_token).await?;

    let api_key = state
        .config
        .backup
        .api_key
        .as_deref()
        .filter(|k| !k.is_empty())
        .map(|k| k.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string().replace('-', ""));

    let backup_address = determine_backup_address(&state.config, &headers);
    tracing::info!(
        backup_address = %backup_address,
        base_url = %state.config.server.base_url,
        "Determined backup address for primary registration"
    );

    let host_display = headers
        .get("Host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or(&backup_address);

    let backup_server_id = register_backup_on_primary(
        &client,
        &base_url,
        &remote_token,
        &backup_address,
        host_display,
        &api_key,
    )
    .await?;

    let user_id = create_local_admin(
        &state.pool,
        &client,
        &base_url,
        &remote_token,
        &req.username,
        &req.password,
        state.config.auth.bcrypt_cost,
        state.config.storage.default_quota_bytes,
    )
    .await?;

    configure_backup_mode(&state.pool, &base_url, &api_key).await?;

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

    trigger_initial_sync(&base_url, &remote_token, backup_server_id);

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "message": "Paired successfully! This server is now a backup.",
            "user_id": user_id,
            "username": req.username,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "main_server_url": base_url,
        })),
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
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
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
        .map_err(|e| {
            AppError::BadRequest(format!(
                "Cannot reach the backup server at {}: {}",
                base_url, e
            ))
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::BadRequest(format!(
            "Backup server rejected the credentials (HTTP {}): {}",
            status, body
        )));
    }

    let login_data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse login response: {}", e)))?;

    let access_token = login_data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AppError::Internal("No access_token in backup server login response".into())
        })?;

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
        api_key = mode_data
            .get("api_key")
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
        base_url,
        photo_count
    );

    Ok(Json(VerifyBackupResponse {
        address: req.address.trim().to_string(),
        name: server_name,
        version,
        api_key,
        photo_count,
    }))
}
