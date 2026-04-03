//! Password change and verification handlers.

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use std::net::SocketAddr;

use crate::audit::{self, AuditEvent};
use crate::error::AppError;
use crate::ratelimit::extract_client_ip;
use crate::state::AppState;

use super::middleware::AuthUser;
use super::models::*;
use super::validation::validate_password;

/// Change password — requires current password + new password.
/// On success, revokes ALL existing refresh tokens for the user,
/// forcing re-authentication on every device.
///
/// **Note:** This endpoint is rate-limited (login limiter) but does NOT
/// trigger account lockout on wrong-password attempts. An attacker with
/// a stolen access token could brute-force the current password within
/// the rate limit window (10 req/min).
pub async fn change_password(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    auth: AuthUser,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AppError> {
    let ip = extract_client_ip(&headers, state.config.server.trust_proxy, Some(peer));
    state.rate_limiters.login.check(ip)?;

    validate_password(&req.new_password)?;

    let current_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;

    let pw = req.current_password.clone();
    let hash = current_hash.clone();
    let valid = tokio::task::spawn_blocking(move || bcrypt::verify(&pw, &hash))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
        .map_err(|e| AppError::Internal(format!("bcrypt error: {}", e)))?;

    if !valid {
        audit::log(
            &state,
            AuditEvent::LoginFailure,
            Some(&auth.user_id),
            &headers,
            Some(serde_json::json!({ "reason": "wrong_password_on_change" })),
        )
        .await;
        return Err(AppError::Unauthorized(
            "Current password is incorrect".into(),
        ));
    }

    let new_pw = req.new_password.clone();
    let cost = state.config.auth.bcrypt_cost;
    let new_hash = tokio::task::spawn_blocking(move || bcrypt::hash(&new_pw, cost))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;

    // Begin transaction — UPDATE password + revoke tokens must be atomic.
    // If the password is updated but token revocation fails, old sessions
    // continue using the old password's tokens with no forced re-auth.
    let mut tx = state.pool.begin().await?;

    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(&new_hash)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    // Revoke ALL refresh tokens — force re-authentication everywhere
    sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE user_id = ?")
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    audit::log(
        &state,
        AuditEvent::PasswordChanged,
        Some(&auth.user_id),
        &headers,
        None,
    )
    .await;

    tracing::info!("Password changed for user {}", auth.user_id);
    Ok(StatusCode::OK)
}

/// POST /api/auth/verify-password
/// Verify the current user's password without any side effects.
/// Used by clients to gate sensitive actions (e.g. enabling biometric lock).
///
/// **Note:** Rate-limited (login limiter) but does NOT trigger account
/// lockout on failure. Same brute-force caveat as [`change_password`].
pub async fn verify_password(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    auth: AuthUser,
    Json(req): Json<VerifyPasswordRequest>,
) -> Result<StatusCode, AppError> {
    let ip = extract_client_ip(&headers, state.config.server.trust_proxy, Some(peer));
    state.rate_limiters.login.check(ip)?;

    let current_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;

    let pw = req.password.clone();
    let hash = current_hash.clone();
    let valid = tokio::task::spawn_blocking(move || bcrypt::verify(&pw, &hash))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
        .map_err(|e| AppError::Internal(format!("bcrypt error: {}", e)))?;

    if !valid {
        return Err(AppError::Unauthorized("Password is incorrect".into()));
    }

    Ok(StatusCode::OK)
}
