//! Login, registration, token refresh, and logout handlers.

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use std::net::SocketAddr;
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::error::AppError;
use crate::ratelimit::extract_client_ip;
use crate::state::AppState;

use super::lockout::{check_account_lockout, clear_failed_logins, record_failed_login};
use super::models::*;
use super::tokens::{create_jwt, hash_token, issue_tokens};
use super::totp::{verify_backup_code, verify_totp_code};
use super::validation::{validate_password, validate_username};

/// POST /api/auth/register — create a new user account.
///
/// Rate-limited, validates username/password, hashes with bcrypt.
pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), AppError> {
    let ip = extract_client_ip(&headers, state.config.server.trust_proxy, Some(peer));
    state.rate_limiters.register.check(ip)?;

    if !state.config.auth.allow_registration {
        return Err(AppError::Forbidden("Registration is disabled".into()));
    }

    validate_username(&req.username)?;
    validate_password(&req.password)?;

    let existing = sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE username = ?")
        .bind(&req.username)
        .fetch_optional(&state.pool)
        .await?;

    if existing.is_some() {
        // Security: timing-safe — do a dummy hash so timing is the same.
        // Offloaded to the blocking threadpool so the tokio worker is free.
        let cost = state.config.auth.bcrypt_cost;
        let _ =
            tokio::task::spawn_blocking(move || bcrypt::hash("dummy_password_for_timing", cost))
                .await;
        return Err(AppError::Conflict("Username already taken".into()));
    }

    let user_id = Uuid::new_v4().to_string();
    let password_clone = req.password.clone();
    let cost = state.config.auth.bcrypt_cost;
    let password_hash = tokio::task::spawn_blocking(move || bcrypt::hash(&password_clone, cost))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes) VALUES (?, ?, ?, ?, ?)",
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
        Some(serde_json::json!({ "username": req.username })),
    )
    .await;

    tracing::info!("User registered: {} ({})", req.username, user_id);

    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            user_id,
            username: req.username,
        }),
    ))
}

/// POST /api/auth/login — authenticate with username + password.
///
/// If 2FA is enabled for this user, returns a short-lived TOTP session token
/// instead of full tokens — the client must complete login via `login_totp`.
pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ip = extract_client_ip(&headers, state.config.server.trust_proxy, Some(peer));
    state.rate_limiters.login.check(ip)?;

    // Timing-safe: always do a password check even if user doesn't exist
    let user = sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, created_at, storage_quota_bytes, totp_secret, totp_enabled FROM users WHERE username = ?",
    )
    .bind(&req.username)
    .fetch_optional(&state.pool)
    .await?;

    let user = match user {
        Some(u) => u,
        None => {
            // Timing-safe: always do a password check even if user doesn't exist.
            // Offloaded to blocking threadpool.
            let pw = req.password.clone();
            let _ = tokio::task::spawn_blocking(move || {
                bcrypt::verify(
                    &pw,
                    "$2b$12$LJ3m9blCPMEtJDZk4CYOqe4CIH55aN38bwSqggfgA1mJm/kzbyPhK",
                )
            })
            .await;
            audit::log(
                &state,
                AuditEvent::LoginFailure,
                None,
                &headers,
                Some(serde_json::json!({ "reason": "user_not_found" })),
            )
            .await;
            return Err(AppError::Unauthorized(
                "Invalid username or password".into(),
            ));
        }
    };

    check_account_lockout(&state, &user.id).await?;

    let pw = req.password.clone();
    let hash = user.password_hash.clone();
    let valid = tokio::task::spawn_blocking(move || bcrypt::verify(&pw, &hash))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
        .map_err(|e| AppError::Internal(format!("bcrypt error: {}", e)))?;

    if !valid {
        record_failed_login(&state, &user.id, &headers).await;

        audit::log(
            &state,
            AuditEvent::LoginFailure,
            Some(&user.id),
            &headers,
            Some(serde_json::json!({ "reason": "invalid_password" })),
        )
        .await;

        return Err(AppError::Unauthorized(
            "Invalid username or password".into(),
        ));
    }

    clear_failed_logins(&state, &user.id).await;

    if user.totp_enabled {
        let token = create_jwt(
            &user.id,
            true,
            300, // 5-minute TOTP window
            &state.config.auth.jwt_secret,
            "", // role not needed for TOTP session tokens
        )?;
        return Ok(Json(serde_json::json!({
            "requires_totp": true,
            "totp_session_token": token
        })));
    }

    let (access_token, refresh_token) = issue_tokens(&state, &user.id).await?;

    audit::log(
        &state,
        AuditEvent::LoginSuccess,
        Some(&user.id),
        &headers,
        None,
    )
    .await;

    Ok(Json(serde_json::json!({
        "access_token": access_token,
        "refresh_token": refresh_token,
        "expires_in": state.config.auth.access_token_ttl_secs
    })))
}

/// POST /api/auth/login/totp — complete login by providing a TOTP code or backup code.
///
/// Validates the short-lived TOTP session token issued by `login`, then verifies
/// the 6-digit TOTP code (or single-use backup code) before issuing full tokens.
pub async fn login_totp(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<TotpLoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let ip = extract_client_ip(&headers, state.config.server.trust_proxy, Some(peer));
    state.rate_limiters.totp.check(ip)?;

    let key = DecodingKey::from_secret(state.config.auth.jwt_secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp", "sub"]);

    let token_data = decode::<Claims>(&req.totp_session_token, &key, &validation)
        .map_err(|e| AppError::Unauthorized(format!("Invalid TOTP session token: {}", e)))?;

    if !token_data.claims.totp_required {
        return Err(AppError::BadRequest("Not a TOTP session token".into()));
    }

    let user_id = &token_data.claims.sub;

    check_account_lockout(&state, user_id).await?;

    let user = sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, created_at, storage_quota_bytes, totp_secret, totp_enabled FROM users WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;

    let totp_secret = user
        .totp_secret
        .as_ref()
        .ok_or_else(|| AppError::Internal("TOTP enabled but no secret found".into()))?;

    if let Some(code) = &req.totp_code {
        match verify_totp_code(
            totp_secret,
            code,
            &state.config.server.base_url,
            &user.username,
        ) {
            Ok(()) => {
                audit::log(
                    &state,
                    AuditEvent::TotpLoginSuccess,
                    Some(user_id),
                    &headers,
                    None,
                )
                .await;
            }
            Err(e) => {
                record_failed_login(&state, user_id, &headers).await;
                audit::log(
                    &state,
                    AuditEvent::TotpLoginFailure,
                    Some(user_id),
                    &headers,
                    None,
                )
                .await;
                return Err(e);
            }
        }
    } else if let Some(backup) = &req.backup_code {
        verify_backup_code(&state, user_id, backup).await?;
        audit::log(
            &state,
            AuditEvent::BackupCodeUsed,
            Some(user_id),
            &headers,
            None,
        )
        .await;
    } else {
        return Err(AppError::BadRequest(
            "Either totp_code or backup_code is required".into(),
        ));
    }

    clear_failed_logins(&state, user_id).await;

    let (access_token, refresh_token) = issue_tokens(&state, user_id).await?;

    audit::log(
        &state,
        AuditEvent::LoginSuccess,
        Some(user_id),
        &headers,
        Some(serde_json::json!({ "method": "totp" })),
    )
    .await;

    Ok(Json(LoginResponse {
        access_token,
        refresh_token,
        expires_in: state.config.auth.access_token_ttl_secs,
    }))
}

/// POST /api/auth/refresh — exchange a refresh token for a new access/refresh pair.
///
/// Implements refresh-token rotation: the old token is revoked on use.
/// Reuse of a revoked token is treated as potential theft — *all* tokens
/// for the user are revoked, forcing re-authentication everywhere.
///
/// Rate-limited with the `general` limiter (100 req/60s per IP) to prevent
/// abuse from issuing unlimited refresh-token rotations.
pub async fn refresh(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, AppError> {
    let ip = extract_client_ip(&headers, state.config.server.trust_proxy, Some(peer));
    state.rate_limiters.general.check(ip)?;
    let token_hash = hash_token(&req.refresh_token);

    let row = sqlx::query_as::<_, (String, String, bool)>(
        "SELECT id, user_id, revoked != 0 as revoked FROM refresh_tokens WHERE token_hash = ? AND expires_at > ?",
    )
    .bind(&token_hash)
    .bind(Utc::now().to_rfc3339())
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid or expired refresh token".into()))?;

    let (token_id, user_id, revoked) = row;

    // Refresh-token rotation theft detection: if a revoked token is replayed,
    // either an attacker or the legitimate user has a stolen token — revoke
    // everything to contain the breach.
    if revoked {
        tracing::warn!(
            user_id = user_id,
            "Revoked refresh token reused — possible token theft, revoking all tokens"
        );
        sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE user_id = ?")
            .bind(&user_id)
            .execute(&state.pool)
            .await?;

        audit::log(
            &state,
            AuditEvent::LoginFailure,
            Some(&user_id),
            &headers,
            Some(serde_json::json!({ "reason": "revoked_token_reuse" })),
        )
        .await;

        return Err(AppError::Unauthorized(
            "Refresh token has been revoked. Please log in again.".into(),
        ));
    }

    // Rotate: revoke old, issue new pair
    sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE id = ?")
        .bind(&token_id)
        .execute(&state.pool)
        .await?;

    let (access_token, new_refresh_token) = issue_tokens(&state, &user_id).await?;

    audit::log(
        &state,
        AuditEvent::TokenRefresh,
        Some(&user_id),
        &headers,
        None,
    )
    .await;

    Ok(Json(RefreshResponse {
        access_token,
        refresh_token: new_refresh_token,
        expires_in: state.config.auth.access_token_ttl_secs,
    }))
}

/// POST /api/auth/logout — revoke the given refresh token.
///
/// Always returns `204 NO_CONTENT` regardless of whether the token existed
/// or was already revoked. This prevents token-hash enumeration.
pub async fn logout(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LogoutRequest>,
) -> Result<StatusCode, AppError> {
    let ip = extract_client_ip(&headers, state.config.server.trust_proxy, Some(peer));
    state.rate_limiters.general.check(ip)?;
    let token_hash = hash_token(&req.refresh_token);

    let user_id: Option<String> =
        sqlx::query_scalar("SELECT user_id FROM refresh_tokens WHERE token_hash = ?")
            .bind(&token_hash)
            .fetch_optional(&state.pool)
            .await?;

    sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE token_hash = ?")
        .bind(&token_hash)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state,
        AuditEvent::Logout,
        user_id.as_deref(),
        &headers,
        None,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
