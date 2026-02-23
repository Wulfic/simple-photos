use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::error::AppError;
use crate::ratelimit::extract_client_ip;
use crate::state::AppState;

use super::middleware::AuthUser;
use super::models::*;

// ── Security constants ──────────────────────────────────────────────────────

/// Maximum failed login attempts before account lockout.
const MAX_LOGIN_ATTEMPTS: i32 = 5;

/// Account lockout duration after exceeding max attempts.
const LOCKOUT_DURATION_MINS: i64 = 15;

/// Maximum password length to prevent DoS via bcrypt (bcrypt truncates at 72 bytes anyway)
const MAX_PASSWORD_LENGTH: usize = 128;

/// Username validation: alphanumeric + underscore, 3–50 chars
fn validate_username(username: &str) -> Result<(), AppError> {
    if username.len() < 3 || username.len() > 50 {
        return Err(AppError::BadRequest(
            "Username must be between 3 and 50 characters".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::BadRequest(
            "Username may only contain letters, numbers, and underscores".into(),
        ));
    }
    Ok(())
}

/// Password validation: length + complexity
fn validate_password(password: &str) -> Result<(), AppError> {
    if password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".into(),
        ));
    }
    if password.len() > MAX_PASSWORD_LENGTH {
        return Err(AppError::BadRequest(
            format!("Password must not exceed {} characters", MAX_PASSWORD_LENGTH),
        ));
    }
    // Require at least one uppercase, one lowercase, one digit
    let has_upper = password.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = password.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    if !has_upper || !has_lower || !has_digit {
        return Err(AppError::BadRequest(
            "Password must contain at least one uppercase letter, one lowercase letter, and one digit".into(),
        ));
    }
    Ok(())
}

// ── Account lockout helpers ─────────────────────────────────────────────────

async fn check_account_lockout(
    state: &AppState,
    user_id: &str,
) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, (i32, Option<String>)>(
        "SELECT failed_attempts, lockout_until FROM account_lockouts WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;

    if let Some((_attempts, lockout_until)) = row {
        if let Some(until) = lockout_until {
            if let Ok(lock_time) = chrono::DateTime::parse_from_rfc3339(&until) {
                let lock_time_utc = lock_time.with_timezone(&Utc);
                if Utc::now() < lock_time_utc {
                    let remaining = (lock_time_utc - Utc::now()).num_seconds();
                    return Err(AppError::Forbidden(format!(
                        "Account is temporarily locked. Try again in {} seconds.",
                        remaining.max(0)
                    )));
                }
                // Lockout expired — reset
                sqlx::query(
                    "UPDATE account_lockouts SET failed_attempts = 0, lockout_until = NULL WHERE user_id = ?",
                )
                .bind(user_id)
                .execute(&state.pool)
                .await?;
            }
        }
    }
    Ok(())
}

async fn record_failed_login(
    state: &AppState,
    user_id: &str,
    headers: &HeaderMap,
) {
    let now = Utc::now().to_rfc3339();

    // Upsert: increment or insert
    let result = sqlx::query(
        "INSERT INTO account_lockouts (user_id, failed_attempts, last_attempt_at) \
         VALUES (?, 1, ?) \
         ON CONFLICT(user_id) DO UPDATE SET \
           failed_attempts = failed_attempts + 1, \
           last_attempt_at = ?",
    )
    .bind(user_id)
    .bind(&now)
    .bind(&now)
    .execute(&state.pool)
    .await;

    if let Err(e) = result {
        tracing::error!("Failed to record failed login: {}", e);
        return;
    }

    // Check if we should lock the account
    let attempts: Option<i32> = sqlx::query_scalar(
        "SELECT failed_attempts FROM account_lockouts WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    if let Some(count) = attempts {
        if count >= MAX_LOGIN_ATTEMPTS {
            let lockout_until =
                (Utc::now() + chrono::Duration::minutes(LOCKOUT_DURATION_MINS)).to_rfc3339();

            let _ = sqlx::query(
                "UPDATE account_lockouts SET lockout_until = ? WHERE user_id = ?",
            )
            .bind(&lockout_until)
            .bind(user_id)
            .execute(&state.pool)
            .await;

            tracing::warn!(
                user_id = user_id,
                attempts = count,
                "Account locked after {} failed attempts",
                count
            );

            audit::log(
                &state.pool,
                AuditEvent::AccountLocked,
                Some(user_id),
                headers,
                Some(serde_json::json!({ "attempts": count })),
            )
            .await;
        }
    }
}

async fn clear_failed_logins(state: &AppState, user_id: &str) {
    let _ = sqlx::query("DELETE FROM account_lockouts WHERE user_id = ?")
        .bind(user_id)
        .execute(&state.pool)
        .await;
}

// ── Handlers ────────────────────────────────────────────────────────────────

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), AppError> {
    // Rate limit
    let ip = extract_client_ip(&headers);
    state.rate_limiters.register.check(ip)?;

    if !state.config.auth.allow_registration {
        return Err(AppError::Forbidden("Registration is disabled".into()));
    }

    // Validate inputs
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
    // Rate limit
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
            // Perform a dummy bcrypt verify to prevent timing attacks
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

    // Check account lockout before password verification
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

    // Password correct — clear lockout counters
    clear_failed_logins(&state, &user.id).await;

    if user.totp_enabled {
        let token = create_jwt(
            &user.id,
            true,
            300, // 5-minute TOTP window
            &state.config.auth.jwt_secret,
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
    // Rate limit TOTP separately — only 1M possible codes
    let ip = extract_client_ip(&headers);
    state.rate_limiters.totp.check(ip)?;

    let key = DecodingKey::from_secret(state.config.auth.jwt_secret.as_bytes());
    // Strictly require HS256 — prevent algorithm confusion attacks
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp", "sub"]);

    let token_data = decode::<Claims>(&req.totp_session_token, &key, &validation)
        .map_err(|e| AppError::Unauthorized(format!("Invalid TOTP session token: {}", e)))?;

    if !token_data.claims.totp_required {
        return Err(AppError::BadRequest("Not a TOTP session token".into()));
    }

    let user_id = &token_data.claims.sub;

    // Check lockout before TOTP attempt
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

    // Successful TOTP — clear lockout counters
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
        // A revoked token being used is suspicious — possible token theft.
        // Revoke ALL tokens for this user as a precaution.
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

    // Get user_id before revoking for audit log
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

    // Delete old backup codes
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

// ── Helper functions ────────────────────────────────────────────────────────

fn create_jwt(
    user_id: &str,
    totp_required: bool,
    ttl_secs: u64,
    secret: &str,
) -> Result<String, AppError> {
    let exp = (Utc::now().timestamp() as u64 + ttl_secs) as usize;
    let jti = Uuid::new_v4().to_string();
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        jti,
        totp_required,
    };
    // Explicitly HS256 — prevent algorithm confusion attacks
    let header = Header::new(Algorithm::HS256);
    encode(
        &header,
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encoding error: {}", e)))
}

async fn issue_tokens(
    state: &AppState,
    user_id: &str,
) -> Result<(String, String), AppError> {
    let access_token = create_jwt(
        user_id,
        false,
        state.config.auth.access_token_ttl_secs,
        &state.config.auth.jwt_secret,
    )?;

    let raw_refresh = Uuid::new_v4().to_string();
    let refresh_hash = hash_token(&raw_refresh);
    let expires_at = Utc::now()
        + chrono::Duration::days(state.config.auth.refresh_token_ttl_days as i64);

    sqlx::query(
        "INSERT INTO refresh_tokens (id, user_id, token_hash, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(&refresh_hash)
    .bind(expires_at.to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&state.pool)
    .await?;

    Ok((access_token, raw_refresh))
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn verify_totp_code(
    secret_b32: &str,
    code: &str,
    _issuer: &str,
    account: &str,
) -> Result<(), AppError> {
    // Validate TOTP code format: must be exactly 6 digits
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::BadRequest(
            "TOTP code must be exactly 6 digits".into(),
        ));
    }

    let secret = totp_rs::Secret::Encoded(secret_b32.to_string());
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret
            .to_bytes()
            .map_err(|e| AppError::Internal(format!("TOTP secret error: {}", e)))?,
        Some("SimplePhotos".to_string()),
        account.to_string(),
    )
    .map_err(|e| AppError::Internal(format!("TOTP error: {}", e)))?;

    if totp
        .check_current(code)
        .map_err(|e| AppError::Internal(format!("TOTP time error: {}", e)))?
    {
        Ok(())
    } else {
        Err(AppError::Unauthorized("Invalid TOTP code".into()))
    }
}

async fn verify_backup_code(
    state: &AppState,
    user_id: &str,
    backup_code: &str,
) -> Result<(), AppError> {
    let code_hash = hex::encode(Sha256::digest(backup_code.as_bytes()));

    let row = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM totp_backup_codes WHERE user_id = ? AND code_hash = ? AND used = 0",
    )
    .bind(user_id)
    .bind(&code_hash)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid or already used backup code".into()))?;

    sqlx::query("UPDATE totp_backup_codes SET used = 1 WHERE id = ?")
        .bind(&row.0)
        .execute(&state.pool)
        .await?;

    Ok(())
}
