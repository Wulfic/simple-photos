//! 2FA (TOTP) management handlers: status, setup, confirm, disable.

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::error::AppError;
use crate::ratelimit::extract_client_ip;
use crate::state::AppState;

use super::middleware::AuthUser;
use super::models::*;
use super::totp::verify_totp_code;

/// GET /api/auth/2fa/status — check if 2FA is enabled for the current user.
pub async fn get_2fa_status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let enabled: bool = sqlx::query_scalar("SELECT totp_enabled != 0 FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(serde_json::json!({ "totp_enabled": enabled })))
}

/// POST /api/auth/2fa/setup — generate a TOTP secret and backup codes.
///
/// Does **not** enable 2FA yet — the client must confirm with [`confirm_2fa`]
/// by providing a valid code from their authenticator app.
pub async fn setup_2fa(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: AuthUser,
) -> Result<Json<TotpSetupResponse>, AppError> {
    let user = sqlx::query_as::<_, (String, bool)>(
        "SELECT username, totp_enabled != 0 FROM users WHERE id = ?",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let (username, totp_enabled) = user;
    if totp_enabled {
        return Err(AppError::BadRequest("2FA is already enabled".into()));
    }

    let secret = totp_rs::Secret::generate_secret();
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret
            .to_bytes()
            .map_err(|e| AppError::Internal(format!("TOTP secret error: {}", e)))?,
        Some("SimplePhotos".to_string()),
        username.clone(),
    )
    .map_err(|e| AppError::Internal(format!("TOTP creation error: {}", e)))?;

    let otpauth_uri = totp.get_url();

    let secret_b32 = secret.to_encoded().to_string();

    // Generate 10 backup codes using CSPRNG
    let backup_codes: Vec<String> = {
        let mut rng = rand::thread_rng();
        (0..10)
            .map(|_| {
                (0..8)
                    .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
                    .collect()
            })
            .collect()
    };

    // Wrap secret + backup-code writes in a transaction so a crash between
    // DELETE and INSERTs can't leave the user with a TOTP secret but no
    // backup codes.
    let mut tx = state.pool.begin().await?;

    sqlx::query("UPDATE users SET totp_secret = ? WHERE id = ?")
        .bind(&secret_b32)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    for code in &backup_codes {
        let code_hash = hex::encode(Sha256::digest(code.as_bytes()));
        sqlx::query("INSERT INTO totp_backup_codes (id, user_id, code_hash) VALUES (?, ?, ?)")
            .bind(Uuid::new_v4().to_string())
            .bind(&auth.user_id)
            .bind(&code_hash)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    audit::log(
        &state,
        AuditEvent::TotpSetup,
        Some(&auth.user_id),
        &headers,
        None,
    )
    .await;

    Ok(Json(TotpSetupResponse {
        otpauth_uri,
        backup_codes,
    }))
}

/// POST /api/auth/2fa/confirm — verify a TOTP code to activate 2FA.
///
/// The user must have called `setup_2fa` first to generate a secret.
pub async fn confirm_2fa(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    auth: AuthUser,
    Json(req): Json<TotpConfirmRequest>,
) -> Result<StatusCode, AppError> {
    // Rate-limit TOTP confirmation to prevent brute-force of 6-digit codes
    let ip = extract_client_ip(&headers, state.config.server.trust_proxy, Some(peer));
    state.rate_limiters.totp.check(ip)?;

    let user = sqlx::query_as::<_, (Option<String>, bool)>(
        "SELECT totp_secret, totp_enabled != 0 FROM users WHERE id = ?",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let (totp_secret, totp_enabled) = user;
    if totp_enabled {
        return Err(AppError::BadRequest("2FA is already enabled".into()));
    }

    let secret = totp_secret.ok_or_else(|| {
        AppError::BadRequest("2FA setup not initiated. Call /2fa/setup first".into())
    })?;

    let username = sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;

    verify_totp_code(
        &secret,
        &req.totp_code,
        &state.config.server.base_url,
        &username,
    )?;

    sqlx::query("UPDATE users SET totp_enabled = 1 WHERE id = ?")
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state,
        AuditEvent::TotpEnabled,
        Some(&auth.user_id),
        &headers,
        None,
    )
    .await;

    tracing::info!("2FA enabled for user {}", auth.user_id);
    Ok(StatusCode::OK)
}

/// POST /api/auth/2fa/disable — turn off 2FA (requires a valid TOTP code to confirm).
pub async fn disable_2fa(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    auth: AuthUser,
    Json(req): Json<TotpDisableRequest>,
) -> Result<StatusCode, AppError> {
    // Rate-limit TOTP verification to prevent brute-force of 6-digit codes
    let ip = extract_client_ip(&headers, state.config.server.trust_proxy, Some(peer));
    state.rate_limiters.totp.check(ip)?;

    let user = sqlx::query_as::<_, (Option<String>, bool, String)>(
        "SELECT totp_secret, totp_enabled != 0, username FROM users WHERE id = ?",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let (totp_secret, totp_enabled, username) = user;
    if !totp_enabled {
        return Err(AppError::BadRequest("2FA is not enabled".into()));
    }

    let secret =
        totp_secret.ok_or_else(|| AppError::Internal("TOTP enabled but no secret".into()))?;

    verify_totp_code(
        &secret,
        &req.totp_code,
        &state.config.server.base_url,
        &username,
    )?;

    // Transaction: disable 2FA flag + delete backup codes atomically so a
    // crash can't leave orphaned codes in the table.
    let mut tx = state.pool.begin().await?;

    sqlx::query("UPDATE users SET totp_enabled = 0, totp_secret = NULL WHERE id = ?")
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    audit::log(
        &state,
        AuditEvent::TotpDisabled,
        Some(&auth.user_id),
        &headers,
        None,
    )
    .await;

    tracing::info!("2FA disabled for user {}", auth.user_id);
    Ok(StatusCode::NO_CONTENT)
}
