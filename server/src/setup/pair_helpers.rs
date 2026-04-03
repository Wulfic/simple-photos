//! Helper functions for the backup pairing and restore-verification flow.
//!
//! Extracted from [`super::pair`] to keep the HTTP handlers lean and the
//! supporting logic (auth, registration, address detection) testable in
//! isolation.

use axum::http::HeaderMap;
use chrono::Utc;
use uuid::Uuid;

use crate::error::AppError;

// ── Types ────────────────────────────────────────────────────────────────────

/// Outcome of attempting to authenticate against the primary server.
pub(crate) enum PrimaryAuthOutcome {
    /// Successfully authenticated; contains the access token.
    Authenticated(String),
    /// Primary requires TOTP but no code was provided yet.
    NeedsTotp,
}

// ── URL & Address ────────────────────────────────────────────────────────────

/// Normalize a server URL: ensure it has a scheme and no trailing slash.
pub(crate) fn normalize_server_url(raw: &str) -> String {
    let base = raw.trim().trim_end_matches('/');
    if base.starts_with("http://") || base.starts_with("https://") {
        base.to_string()
    } else {
        format!("http://{}", base)
    }
}

/// Determine the routable address for this backup server as seen from the LAN.
///
/// Uses `config.server.base_url` — the externally-reachable URL the user
/// configured.  Falls back to LAN IP detection when base_url points at
/// localhost/loopback.
pub(crate) fn determine_backup_address(
    config: &crate::config::AppConfig,
    headers: &HeaderMap,
) -> String {
    let base = config.server.base_url.trim_end_matches('/');
    let host_port = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .unwrap_or(base)
        .split('/')
        .next()
        .unwrap_or("");

    if host_port.is_empty() || host_port == "localhost" || host_port.starts_with("127.") {
        let ip = crate::backup::broadcast::get_local_ip().unwrap_or_else(|| {
            headers
                .get("Host")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("unknown-backup-host")
                .to_string()
        });
        if ip.contains(':') {
            ip
        } else {
            format!("{}:{}", ip, config.server.port)
        }
    } else {
        host_port.to_string()
    }
}

// ── Primary Authentication ───────────────────────────────────────────────────

/// Authenticate against a primary server, handling the optional 2FA flow.
///
/// Returns `Authenticated(token)` on success, or `NeedsTotp` if the primary
/// requires a TOTP code that wasn't provided.
pub(crate) async fn authenticate_with_primary(
    client: &reqwest::Client,
    base_url: &str,
    username: &str,
    password: &str,
    totp_code: Option<&str>,
) -> Result<PrimaryAuthOutcome, AppError> {
    let login_url = format!("{}/api/auth/login", base_url);
    let login_body = serde_json::json!({
        "username": username,
        "password": password,
    });

    let resp = client
        .post(&login_url)
        .header("Content-Type", "application/json")
        .json(&login_body)
        .send()
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "Cannot reach the primary server at {}: {}",
                base_url, e
            ))
        })?;

    let status = resp.status();
    let resp_bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        let body = String::from_utf8_lossy(&resp_bytes);
        return Err(AppError::BadRequest(format!(
            "Primary server rejected the credentials (HTTP {}): {}",
            status, body
        )));
    }

    let login_data: serde_json::Value = serde_json::from_slice(&resp_bytes)
        .map_err(|_| AppError::Internal("Failed to parse login response".into()))?;

    if login_data
        .get("requires_totp")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let totp_session_token = login_data
            .get("totp_session_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AppError::Internal(
                    "Primary returned requires_totp without a session token".into(),
                )
            })?;

        let code = match totp_code {
            Some(c) if !c.is_empty() => c,
            _ => return Ok(PrimaryAuthOutcome::NeedsTotp),
        };

        let totp_url = format!("{}/api/auth/login/totp", base_url);
        let totp_body = serde_json::json!({
            "totp_session_token": totp_session_token,
            "totp_code": code,
        });

        let totp_resp = client
            .post(&totp_url)
            .header("Content-Type", "application/json")
            .json(&totp_body)
            .send()
            .await
            .map_err(|e| {
                AppError::BadRequest(format!(
                    "Failed to verify 2FA code with primary server: {}",
                    e
                ))
            })?;

        if !totp_resp.status().is_success() {
            let err_status = totp_resp.status();
            let err_body = totp_resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&err_body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                .unwrap_or_else(|| format!("HTTP {}: {}", err_status, err_body));
            return Err(AppError::BadRequest(format!(
                "2FA verification failed: {}",
                msg
            )));
        }

        let totp_data: serde_json::Value = totp_resp
            .json()
            .await
            .map_err(|_| AppError::Internal("Failed to parse 2FA response".into()))?;

        Ok(PrimaryAuthOutcome::Authenticated(
            totp_data
                .get("access_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    AppError::Internal(
                        "Primary did not return an access token after 2FA verification".into(),
                    )
                })?
                .to_string(),
        ))
    } else {
        Ok(PrimaryAuthOutcome::Authenticated(
            login_data
                .get("access_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    AppError::Internal("Primary server did not return an access token".into())
                })?
                .to_string(),
        ))
    }
}

/// Verify the target is a primary server, not another backup.
///
/// If the mode endpoint is unreachable or returns a non-success status the
/// pairing proceeds — older server versions may not have this endpoint.
pub(crate) async fn verify_primary_is_not_backup(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> Result<(), AppError> {
    let mode_url = format!("{}/api/admin/backup/mode", base_url);
    if let Ok(mode_resp) = client
        .get(&mode_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
    {
        if mode_resp.status().is_success() {
            if let Ok(mode_data) = mode_resp.json::<serde_json::Value>().await {
                let server_mode = mode_data
                    .get("mode")
                    .and_then(|m| m.as_str())
                    .unwrap_or("primary");
                if server_mode != "primary" {
                    return Err(AppError::BadRequest(
                        "The target server is already in backup mode. \
                         You can only pair a backup server to a primary server, \
                         not to another backup server."
                            .into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

// ── Registration & Configuration ─────────────────────────────────────────────

/// Register this backup server on the primary, returning the assigned server ID.
pub(crate) async fn register_backup_on_primary(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    backup_address: &str,
    host_display: &str,
    api_key: &str,
) -> Result<Option<String>, AppError> {
    let register_url = format!("{}/api/admin/backup/servers", base_url);
    let register_body = serde_json::json!({
        "name": format!("Backup Server ({})", host_display),
        "address": backup_address,
        "api_key": api_key,
        "sync_frequency_hours": 24,
    });

    let reg_resp = client
        .post(&register_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&register_body)
        .send()
        .await
        .map_err(|e| {
            AppError::Internal(format!(
                "Failed to connect to primary server to register backup server: {}",
                e
            ))
        })?;

    if !reg_resp.status().is_success() {
        let status = reg_resp.status();
        let err_body = reg_resp.text().await.unwrap_or_default();
        return Err(AppError::BadRequest(format!(
            "Failed to register backup server on primary (HTTP {}): {}",
            status, err_body
        )));
    }

    let reg_data: serde_json::Value = reg_resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse registration response: {}", e)))?;

    Ok(reg_data
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}

/// Create a local admin user mirroring the primary's admin account.
///
/// Fetches the primary user's UUID (so IDs match after sync), hashes the
/// password locally, and inserts the user record.
pub(crate) async fn create_local_admin(
    pool: &sqlx::SqlitePool,
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    username: &str,
    password: &str,
    bcrypt_cost: u32,
    default_quota_bytes: u64,
) -> Result<String, AppError> {
    let user_id = {
        let users_url = format!("{}/api/admin/users", base_url);
        let users_resp = client
            .get(&users_url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .ok();
        let mut primary_id: Option<String> = None;
        if let Some(resp) = users_resp {
            if resp.status().is_success() {
                if let Ok(users) = resp.json::<Vec<serde_json::Value>>().await {
                    primary_id = users
                        .iter()
                        .find(|u| u.get("username").and_then(|v| v.as_str()) == Some(username))
                        .and_then(|u| u.get("id").and_then(|v| v.as_str()))
                        .map(|s| s.to_string());
                }
            }
        }
        primary_id.unwrap_or_else(|| Uuid::new_v4().to_string())
    };

    let password_owned = password.to_string();
    let password_hash =
        tokio::task::spawn_blocking(move || bcrypt::hash(&password_owned, bcrypt_cost))
            .await
            .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
            .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, role) \
         VALUES (?, ?, ?, ?, ?, 'admin')",
    )
    .bind(&user_id)
    .bind(username)
    .bind(&password_hash)
    .bind(&now)
    .bind(default_quota_bytes as i64)
    .execute(pool)
    .await?;

    Ok(user_id)
}

/// Store backup-mode settings in the database.
pub(crate) async fn configure_backup_mode(
    pool: &sqlx::SqlitePool,
    base_url: &str,
    api_key: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('backup_mode', 'backup') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('primary_server_url', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(base_url)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('backup_api_key', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(api_key)
    .execute(pool)
    .await?;

    Ok(())
}

/// Fire-and-forget: ask the primary to push all existing photos to this backup.
pub(crate) fn trigger_initial_sync(base_url: &str, remote_token: &str, server_id: Option<String>) {
    if let Some(server_id) = server_id {
        let sync_url = format!("{}/api/admin/backup/servers/{}/sync", base_url, server_id);
        let remote_token = remote_token.to_string();
        tokio::spawn(async move {
            let sync_client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .danger_accept_invalid_certs(true)
                .build()
                .ok();

            if let Some(c) = sync_client {
                match c
                    .post(&sync_url)
                    .header("Authorization", format!("Bearer {}", remote_token))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        tracing::info!(
                            "Initial sync triggered on primary for backup server {}",
                            server_id
                        );
                    }
                    Ok(resp) => {
                        tracing::warn!(
                            "Initial sync trigger returned HTTP {}: primary will sync on next scheduled run",
                            resp.status()
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Could not trigger initial sync on primary (will sync on schedule): {}",
                            e
                        );
                    }
                }
            }
        });
    } else {
        tracing::warn!(
            "Could not extract backup server ID from primary registration response — \
             initial sync will run on next scheduled cycle"
        );
    }
}
