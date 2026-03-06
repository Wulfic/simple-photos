use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
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

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), AppError> {
    let ip = extract_client_ip(&headers);
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
        // Security: timing-safe — do a dummy hash so timing is the same
        let _ = bcrypt::hash("dummy_password_for_timing", state.config.auth.bcrypt_cost);
        return Err(AppError::Conflict("Username already taken".into()));
    }

    let user_id = Uuid::new_v4().to_string();
    let password_hash =
        bcrypt::hash(&req.password, state.config.auth.bcrypt_cost).map_err(|e| {
            AppError::Internal(format!("Failed to hash password: {}", e))
        })?;
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
        &state.pool,
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

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ip = extract_client_ip(&headers);
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
            let _ = bcrypt::verify(
                &req.password,
                "$2b$12$LJ3m9blCPMEtJDZk4CYOqe4CIH55aN38bwSqggfgA1mJm/kzbyPhK",
            );
            audit::log(
                &state.pool,
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

    let valid = bcrypt::verify(&req.password, &user.password_hash)
        .map_err(|e| AppError::Internal(format!("bcrypt error: {}", e)))?;

    if !valid {
        record_failed_login(&state, &user.id, &headers).await;

        audit::log(
            &state.pool,
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
        &state.pool,
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

pub async fn login_totp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TotpLoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let ip = extract_client_ip(&headers);
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
        match verify_totp_code(totp_secret, code, &state.config.server.base_url, &user.username) {
            Ok(()) => {
                audit::log(
                    &state.pool,
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
                    &state.pool,
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
            &state.pool,
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
        &state.pool,
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

pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, AppError> {
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
            &state.pool,
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
        &state.pool,
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

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LogoutRequest>,
) -> Result<StatusCode, AppError> {
    let token_hash = hash_token(&req.refresh_token);

    let user_id: Option<String> = sqlx::query_scalar(
        "SELECT user_id FROM refresh_tokens WHERE token_hash = ?",
    )
    .bind(&token_hash)
    .fetch_optional(&state.pool)
    .await?;

    sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE token_hash = ?")
        .bind(&token_hash)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state.pool,
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
    let enabled: bool = sqlx::query_scalar(
        "SELECT totp_enabled != 0 FROM users WHERE id = ?",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({ "totp_enabled": enabled })))
}

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
        secret.to_bytes().map_err(|e| AppError::Internal(format!("TOTP secret error: {}", e)))?,
        Some("SimplePhotos".to_string()),
        username.clone(),
    )
    .map_err(|e| AppError::Internal(format!("TOTP creation error: {}", e)))?;

    let otpauth_uri = totp.get_url();

    let secret_b32 = secret.to_encoded().to_string();
    sqlx::query("UPDATE users SET totp_secret = ? WHERE id = ?")
        .bind(&secret_b32)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

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

    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    for code in &backup_codes {
        let code_hash = hex::encode(Sha256::digest(code.as_bytes()));
        sqlx::query("INSERT INTO totp_backup_codes (id, user_id, code_hash) VALUES (?, ?, ?)")
            .bind(Uuid::new_v4().to_string())
            .bind(&auth.user_id)
            .bind(&code_hash)
            .execute(&state.pool)
            .await?;
    }

    audit::log(
        &state.pool,
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

pub async fn confirm_2fa(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: AuthUser,
    Json(req): Json<TotpConfirmRequest>,
) -> Result<StatusCode, AppError> {
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

    verify_totp_code(&secret, &req.totp_code, &state.config.server.base_url, &username)?;

    sqlx::query("UPDATE users SET totp_enabled = 1 WHERE id = ?")
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state.pool,
        AuditEvent::TotpEnabled,
        Some(&auth.user_id),
        &headers,
        None,
    )
    .await;

    tracing::info!("2FA enabled for user {}", auth.user_id);
    Ok(StatusCode::OK)
}

pub async fn disable_2fa(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: AuthUser,
    Json(req): Json<TotpDisableRequest>,
) -> Result<StatusCode, AppError> {
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

    verify_totp_code(&secret, &req.totp_code, &state.config.server.base_url, &username)?;

    sqlx::query("UPDATE users SET totp_enabled = 0, totp_secret = NULL WHERE id = ?")
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state.pool,
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
pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: AuthUser,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AppError> {
    let ip = extract_client_ip(&headers);
    state.rate_limiters.login.check(ip)?;

    validate_password(&req.new_password)?;

    let current_hash: String = sqlx::query_scalar(
        "SELECT password_hash FROM users WHERE id = ?",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let valid = bcrypt::verify(&req.current_password, &current_hash)
        .map_err(|e| AppError::Internal(format!("bcrypt error: {}", e)))?;

    if !valid {
        audit::log(
            &state.pool,
            AuditEvent::LoginFailure,
            Some(&auth.user_id),
            &headers,
            Some(serde_json::json!({ "reason": "wrong_password_on_change" })),
        )
        .await;
        return Err(AppError::Unauthorized("Current password is incorrect".into()));
    }

    let new_hash = bcrypt::hash(&req.new_password, state.config.auth.bcrypt_cost)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;

    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(&new_hash)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    // Revoke ALL refresh tokens — force re-authentication everywhere
    sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE user_id = ?")
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state.pool,
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
pub async fn verify_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: AuthUser,
    Json(req): Json<VerifyPasswordRequest>,
) -> Result<StatusCode, AppError> {
    let ip = extract_client_ip(&headers);
    state.rate_limiters.login.check(ip)?;

    let current_hash: String = sqlx::query_scalar(
        "SELECT password_hash FROM users WHERE id = ?",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let valid = bcrypt::verify(&req.password, &current_hash)
        .map_err(|e| AppError::Internal(format!("bcrypt error: {}", e)))?;

    if !valid {
        return Err(AppError::Unauthorized("Password is incorrect".into()));
    }

    Ok(StatusCode::OK)
}
