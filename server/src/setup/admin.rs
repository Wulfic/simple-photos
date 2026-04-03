//! Admin user management and role checking utilities.
//!
//! Provides:
//! - `require_admin()` — guard used by all admin-only endpoints to verify
//!   the caller has the `admin` role.
//! - CRUD user management: create, list, delete users and change roles.
//! - Password resets on behalf of other users.
//! - 2FA setup/reset lives in [`super::admin_2fa`].
//!
//! All mutating operations write to the audit log for traceability.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
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
    if !req
        .username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::BadRequest(
            "Username may only contain letters, numbers, and underscores".into(),
        ));
    }

    // Validate password (shared rules with auth::validation)
    crate::auth::validation::validate_password(&req.password)?;

    // Check for duplicate username
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE username = ?)")
        .bind(&req.username)
        .fetch_one(&state.pool)
        .await?;

    if exists {
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
        &state,
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
        return Err(AppError::BadRequest(
            "Cannot delete your own account".into(),
        ));
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
        &state,
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
        return Err(AppError::BadRequest(
            "Role must be 'admin' or 'user'".into(),
        ));
    }

    // Prevent admin from demoting themselves
    if user_id == auth.user_id && req.role != "admin" {
        return Err(AppError::BadRequest(
            "Cannot demote your own account".into(),
        ));
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
        &state,
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

    tracing::info!(
        "Admin '{}' set user '{}' role to '{}'",
        auth.user_id,
        user_id,
        req.role
    );

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

    // Validate password strength (shared rules with auth::validation)
    crate::auth::validation::validate_password(&req.new_password)?;

    let password_clone = req.new_password.clone();
    let cost = state.config.auth.bcrypt_cost;
    let password_hash = tokio::task::spawn_blocking(move || bcrypt::hash(&password_clone, cost))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
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
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "admin_reset_password",
            "target_user_id": user_id
        })),
    )
    .await;

    tracing::info!(
        "Admin '{}' reset password for user '{}'",
        auth.user_id,
        user_id
    );

    Ok(Json(serde_json::json!({
        "message": "Password reset successfully"
    })))
}
