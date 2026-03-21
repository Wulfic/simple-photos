//! Axum handlers for authentication endpoints.
//!
//! Covers the full auth lifecycle: registration, login (with optional TOTP),
//! token refresh (with rotation + theft detection), logout, 2FA management,
//! and password changes.

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use std::net::SocketAddr;
use chrono::Utc;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::error::AppError;
use crate::ratelimit::extract_client_ip;
use crate::state::AppState;

use super::lockout::{check_account_lockout, clear_failed_logins, record_failed_login};
use super::middleware::AuthUser;
use super::models::*;
use super::tokens::{create_jwt, hash_token, issue_tokens};
use super::totp::{verify_backup_code, verify_totp_code};
use super::validation::{validate_password, validate_username};

// ── Handlers ────────────────────────────────────────────────────────────────

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
