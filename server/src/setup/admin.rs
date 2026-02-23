//! Admin user management and role checking utilities.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::auth::models::TotpSetupResponse;
use crate::error::AppError;
use crate::state::AppState;

/// Helper to check if the requesting user has admin role.
pub async fn require_admin(state: &AppState, auth: &AuthUser) -> Result<(), AppError> {
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::Unauthorized("User not found".into()))?;

    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    /// "admin" or "user" — defaults to "user"
    pub role: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateUserResponse {
    pub user_id: String,
    pub username: String,
    pub role: String,
}

/// Admin-only: Create a new user with a specified role.
///
/// POST /api/admin/users
pub async fn create_user(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<CreateUserResponse>), AppError> {
    require_admin(&state, &auth).await?;

    let role = req.role.as_deref().unwrap_or("user");
    if role != "admin" && role != "user" {
        return Err(AppError::BadRequest(
            "Role must be 'admin' or 'user'".into(),
        ));
    }

    // Validate username
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

    // Validate password
    if req.password.len() < 8 || req.password.len() > 128 {
        return Err(AppError::BadRequest(
            "Password must be between 8 and 128 characters".into(),
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

    // Check for duplicate username
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE username = ?)")
        .bind(&req.username)
        .fetch_one(&state.pool)
        .await?;

    if exists {
        return Err(AppError::Conflict("Username already taken".into()));
    }

    let user_id = Uuid::new_v4().to_string();
    let password_hash =
        bcrypt::hash(&req.password, state.config.auth.bcrypt_cost)
            .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, role) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&user_id)
    .bind(&req.username)
    .bind(&password_hash)
    .bind(&now)
    .bind(state.config.storage.default_quota_bytes as i64)
    .bind(role)
    .execute(&state.pool)
    .await?;

    audit::log(
        &state.pool,
        AuditEvent::Register,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "username": req.username,
            "role": role,
            "method": "admin_create"
        })),
    )
    .await;

    tracing::info!(
        "Admin '{}' created user '{}' with role '{}'",
        auth.user_id,
        req.username,
        role
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateUserResponse {
            user_id,
            username: req.username,
            role: role.to_string(),
        }),
    ))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub role: String,
    pub totp_enabled: bool,
    pub created_at: String,
}

/// Admin-only: List all users.
///
/// GET /api/admin/users
pub async fn list_users(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<UserInfo>>, AppError> {
    require_admin(&state, &auth).await?;

    let users = sqlx::query_as::<_, UserInfo>(
        "SELECT id, username, role, totp_enabled, created_at FROM users ORDER BY created_at ASC",
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(users))
}

// ── Delete user ────────────────────────────────────────────────────────────

/// Admin-only: Delete a user by ID.
///
/// DELETE /api/admin/users/{id}
pub async fn delete_user(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<StatusCode, AppError> {
    require_admin(&state, &auth).await?;

    // Prevent admin from deleting themselves
    if user_id == auth.user_id {
        return Err(AppError::BadRequest("Cannot delete your own account".into()));
    }

    // Verify user exists
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = ?)")
        .bind(&user_id)
        .fetch_one(&state.pool)
        .await?;

    if !exists {
        return Err(AppError::NotFound);
    }

    // Delete the user (cascades refresh_tokens, totp_backup_codes, blobs, etc.)
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "delete_user",
            "target_user_id": user_id
        })),
    )
    .await;

    tracing::info!("Admin '{}' deleted user '{}'", auth.user_id, user_id);

    Ok(StatusCode::NO_CONTENT)
}

// ── Update user role ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateUserRoleRequest {
    pub role: String,
}

/// Admin-only: Update a user's role.
///
/// PUT /api/admin/users/{id}/role
pub async fn update_user_role(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Json(req): Json<UpdateUserRoleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    if req.role != "admin" && req.role != "user" {
        return Err(AppError::BadRequest("Role must be 'admin' or 'user'".into()));
    }

    // Prevent admin from demoting themselves
    if user_id == auth.user_id && req.role != "admin" {
        return Err(AppError::BadRequest("Cannot demote your own account".into()));
    }

    let result = sqlx::query("UPDATE users SET role = ? WHERE id = ?")
        .bind(&req.role)
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "update_user_role",
            "target_user_id": user_id,
            "new_role": req.role
        })),
    )
    .await;

    tracing::info!("Admin '{}' set user '{}' role to '{}'", auth.user_id, user_id, req.role);

    Ok(Json(serde_json::json!({
        "message": "Role updated",
        "user_id": user_id,
        "role": req.role
    })))
}

// ── Admin reset password ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AdminResetPasswordRequest {
    pub new_password: String,
}

/// Admin-only: Reset a user's password.
///
/// PUT /api/admin/users/{id}/password
pub async fn admin_reset_password(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Json(req): Json<AdminResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    // Validate password strength
    if req.new_password.len() < 8 || req.new_password.len() > 128 {
        return Err(AppError::BadRequest(
            "Password must be between 8 and 128 characters".into(),
        ));
    }
    let has_upper = req.new_password.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = req.new_password.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = req.new_password.chars().any(|c| c.is_ascii_digit());
    if !has_upper || !has_lower || !has_digit {
        return Err(AppError::BadRequest(
            "Password must contain at least one uppercase letter, one lowercase letter, and one digit".into(),
        ));
    }

    let password_hash =
        bcrypt::hash(&req.new_password, state.config.auth.bcrypt_cost)
            .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;

    let result = sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(&password_hash)
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    // Revoke all refresh tokens for the user so they're forced to re-login
    sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE user_id = ?")
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "admin_reset_password",
            "target_user_id": user_id
        })),
    )
    .await;

    tracing::info!("Admin '{}' reset password for user '{}'", auth.user_id, user_id);

    Ok(Json(serde_json::json!({
        "message": "Password reset successfully"
    })))
}

// ── Admin reset 2FA ────────────────────────────────────────────────────────

/// Admin-only: Disable 2FA for a user.
///
/// DELETE /api/admin/users/{id}/2fa
pub async fn admin_reset_2fa(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    let result = sqlx::query(
        "UPDATE users SET totp_enabled = 0, totp_secret = NULL WHERE id = ?"
    )
    .bind(&user_id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    // Delete backup codes for this user
    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "admin_reset_2fa",
            "target_user_id": user_id
        })),
    )
    .await;

    tracing::info!("Admin '{}' reset 2FA for user '{}'", auth.user_id, user_id);

    Ok(Json(serde_json::json!({
        "message": "Two-factor authentication disabled for user"
    })))
}

// ── Admin setup 2FA for a user ─────────────────────────────────────────────

/// Admin-only: Generate 2FA TOTP secret and backup codes for a target user.
///
/// POST /api/admin/users/{id}/2fa/setup
pub async fn admin_setup_2fa(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<Json<TotpSetupResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let user = sqlx::query_as::<_, (String, bool)>(
        "SELECT username, totp_enabled != 0 FROM users WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let (username, totp_enabled) = user;
    if totp_enabled {
        return Err(AppError::BadRequest("2FA is already enabled for this user".into()));
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
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    // Generate 10 backup codes
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
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    for code in &backup_codes {
        let code_hash = hex::encode(Sha256::digest(code.as_bytes()));
        sqlx::query("INSERT INTO totp_backup_codes (id, user_id, code_hash) VALUES (?, ?, ?)")
            .bind(Uuid::new_v4().to_string())
            .bind(&user_id)
            .bind(&code_hash)
            .execute(&state.pool)
            .await?;
    }

    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "admin_setup_2fa",
            "target_user_id": user_id
        })),
    )
    .await;

    tracing::info!("Admin '{}' initiated 2FA setup for user '{}'", auth.user_id, user_id);

    Ok(Json(TotpSetupResponse {
        otpauth_uri,
        backup_codes,
    }))
}

// ── Admin confirm 2FA for a user ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AdminConfirm2faRequest {
    pub totp_code: String,
}

/// Admin-only: Confirm 2FA for a target user after admin_setup_2fa.
///
/// POST /api/admin/users/{id}/2fa/confirm
pub async fn admin_confirm_2fa(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Json(req): Json<AdminConfirm2faRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    let user = sqlx::query_as::<_, (Option<String>, bool)>(
        "SELECT totp_secret, totp_enabled != 0 FROM users WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let (totp_secret, totp_enabled) = user;
    if totp_enabled {
        return Err(AppError::BadRequest("2FA is already enabled for this user".into()));
    }

    let secret_b32 = totp_secret.ok_or(
        AppError::BadRequest("2FA setup not initiated. Call admin setup first".into()),
    )?;

    let secret = totp_rs::Secret::Encoded(secret_b32)
        .to_bytes()
        .map_err(|e| AppError::Internal(format!("TOTP decode error: {}", e)))?;

    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret,
        Some("SimplePhotos".to_string()),
        String::new(),
    )
    .map_err(|e| AppError::Internal(format!("TOTP creation error: {}", e)))?;

    if !totp.check_current(&req.totp_code).map_err(|e| AppError::Internal(format!("TOTP error: {}", e)))? {
        return Err(AppError::BadRequest("Invalid TOTP code".into()));
    }

    sqlx::query("UPDATE users SET totp_enabled = 1 WHERE id = ?")
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "admin_confirm_2fa",
            "target_user_id": user_id
        })),
    )
    .await;

    tracing::info!("Admin '{}' confirmed 2FA for user '{}'", auth.user_id, user_id);

    Ok(Json(serde_json::json!({
        "message": "Two-factor authentication enabled for user"
    })))
}
