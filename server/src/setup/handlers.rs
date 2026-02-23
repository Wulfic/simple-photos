//! First-run setup wizard API endpoints.
//!
//! These endpoints are used by the web frontend's setup wizard to bootstrap
//! the application on first run. They allow creating the initial admin user
//! without requiring authentication (since no users exist yet).
//!
//! Security: `POST /api/setup/init` only works when zero users exist in the DB.
//! Once the first user is created, these endpoints become effectively read-only.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::error::AppError;
use crate::state::AppState;

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SetupStatusResponse {
    /// Whether initial setup has been completed (at least one user exists)
    pub setup_complete: bool,
    /// Whether new user registration is currently enabled
    pub registration_open: bool,
    /// Server version
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct InitSetupRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct InitSetupResponse {
    pub user_id: String,
    pub username: String,
    pub message: String,
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// Check if initial setup has been completed.
///
/// This endpoint is public (no auth required) so the web frontend can
/// determine whether to show the setup wizard or the login page.
///
/// Returns:
/// - `setup_complete: false` → Show first-run wizard
/// - `setup_complete: true` → Show normal login
pub async fn status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatusResponse>, AppError> {
    let user_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await?;

    Ok(Json(SetupStatusResponse {
        setup_complete: user_count > 0,
        registration_open: state.config.auth.allow_registration,
        version: "0.1.0".to_string(),
    }))
}

/// Create the first user during initial setup.
///
/// # Security
/// This endpoint ONLY works when the database has zero users.
/// Once any user exists, this returns 403 Forbidden.
///
/// The first user is created with the same validation rules as normal
/// registration (password complexity, username format, etc.).
pub async fn init(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<InitSetupRequest>,
) -> Result<(StatusCode, Json<InitSetupResponse>), AppError> {
    // ── Guard: only works when no users exist ────────────────────────────────
    let user_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await?;

    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed. Use the normal registration endpoint.".into(),
        ));
    }

    // ── Validate username ───────────────────────────────────────────────────
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

    // ── Validate password ───────────────────────────────────────────────────
    if req.password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".into(),
        ));
    }
    if req.password.len() > 128 {
        return Err(AppError::BadRequest(
            "Password must not exceed 128 characters".into(),
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

    // ── Create user ─────────────────────────────────────────────────────────
    let user_id = Uuid::new_v4().to_string();
    let password_hash =
        bcrypt::hash(&req.password, state.config.auth.bcrypt_cost)
            .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, role) VALUES (?, ?, ?, ?, ?, 'admin')",
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
        Some(serde_json::json!({
            "username": req.username,
            "method": "first_run_setup"
        })),
    )
    .await;

    tracing::info!(
        "First-run setup complete: user '{}' created ({})",
        req.username,
        user_id
    );

    Ok((
        StatusCode::CREATED,
        Json(InitSetupResponse {
            user_id,
            username: req.username,
            message: "Setup complete! You can now log in.".into(),
        }),
    ))
}

