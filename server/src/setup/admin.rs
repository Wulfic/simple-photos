//! Admin user management and role checking utilities.

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
