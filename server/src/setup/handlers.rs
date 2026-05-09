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
use crate::auth::middleware::AuthUser;
use crate::auth::validation;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

/// `server_settings` row that flips to `"true"` only when the first-run wizard
/// reaches its final "Go to Gallery" step. Until then the rest of the API is
/// gated and the frontend redirects everything back to `/welcome`.
pub const WIZARD_COMPLETED_KEY: &str = "wizard_completed";

/// Read the wizard-completed flag from `server_settings`. Missing or any
/// value other than the literal string `"true"` is treated as not completed.
pub async fn is_wizard_completed(state: &AppState) -> Result<bool, AppError> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = ?")
            .bind(WIZARD_COMPLETED_KEY)
            .fetch_optional(&state.pool)
            .await?;
    Ok(matches!(value.as_deref(), Some("true")))
}

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SetupStatusResponse {
    /// Whether initial setup has been completed (at least one user exists).
    ///
    /// NOTE: this is a *necessary* condition for the wizard, not a sufficient
    /// one. A user can exist while the wizard is still mid-flight (the user
    /// closed the tab between the account step and the final step). Use
    /// `wizard_completed` for routing decisions instead.
    pub setup_complete: bool,
    /// Whether the first-run setup wizard has been fully finalized
    /// (the admin clicked "Go to Gallery" on the final step).
    ///
    /// While this is `false`, the API returns 403 for non-setup endpoints
    /// and the web frontend forwards every page to `/welcome`.
    pub wizard_completed: bool,
    /// Whether new user registration is currently enabled
    pub registration_open: bool,
    /// Server version
    pub version: String,
    /// Operating mode: "primary" or "backup"
    pub mode: String,
    /// Stable random ID minted on first run and persisted in `server_settings`.
    /// The frontend wizard uses this to detect when the server has been wiped
    /// (e.g. via `reset-server.sh`) so it can discard a stale in-progress
    /// `sessionStorage` step and start the wizard from the welcome screen.
    pub setup_id: String,
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
pub async fn status(State(state): State<AppState>) -> Result<Json<SetupStatusResponse>, AppError> {
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await?;

    let mode: String =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_mode'")
            .fetch_optional(&state.pool)
            .await?
            .unwrap_or_else(|| "primary".to_string());

    // Mint a stable per-instance setup_id on first request and persist it.
    // After a DB wipe the row is gone → a new id is generated, which lets
    // the frontend wizard detect the reset and clear stale sessionStorage.
    let setup_id: String = match sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_settings WHERE key = 'setup_id'",
    )
    .fetch_optional(&state.pool)
    .await?
    {
        Some(v) if !v.is_empty() => v,
        _ => {
            let new_id = Uuid::new_v4().to_string();
            // INSERT OR IGNORE so concurrent first-status calls don't race.
            sqlx::query(
                "INSERT OR IGNORE INTO server_settings (key, value) VALUES ('setup_id', ?)",
            )
            .bind(&new_id)
            .execute(&state.pool)
            .await?;
            // Re-read in case another request inserted first.
            sqlx::query_scalar::<_, String>(
                "SELECT value FROM server_settings WHERE key = 'setup_id'",
            )
            .fetch_one(&state.pool)
            .await
            .unwrap_or(new_id)
        }
    };

    let wizard_completed = is_wizard_completed(&state).await?;

    Ok(Json(SetupStatusResponse {
        setup_complete: user_count > 0,
        wizard_completed,
        registration_open: state.config.auth.allow_registration,
        version: crate::VERSION.to_string(),
        mode,
        setup_id,
    }))
}

// ── Finalize ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct FinalizeResponse {
    pub wizard_completed: bool,
    pub message: String,
}

/// Mark the first-run wizard as fully completed.
///
/// Called by the frontend's `CompleteStep` when the user clicks "Go to
/// Gallery". Until this is called, every non-setup API endpoint returns 403
/// and the SPA forwards every route to `/welcome`.
///
/// # Security
/// - Requires a valid auth token.
/// - Requires admin role (only the admin created during the wizard can
///   finalize). This prevents a hypothetical scenario where a non-admin
///   account exists but somehow reaches this endpoint.
/// - Requires `setup_complete = true` (at least one user must exist).
/// - Idempotent: calling it again on an already-finalized server is a no-op.
pub async fn finalize(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: AuthUser,
) -> Result<Json<FinalizeResponse>, AppError> {
    require_admin(&state, &auth).await?;

    // Belt-and-suspenders: refuse to finalize without any users. (Cannot
    // actually reach this path because AuthUser already requires a real user.)
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await?;
    if user_count == 0 {
        return Err(AppError::BadRequest(
            "Cannot finalize wizard: no users exist.".into(),
        ));
    }

    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES (?, 'true') \
         ON CONFLICT(key) DO UPDATE SET value = 'true'",
    )
    .bind(WIZARD_COMPLETED_KEY)
    .execute(&state.pool)
    .await?;

    audit::log(
        &state,
        AuditEvent::Register,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "event": "wizard_finalized",
        })),
    )
    .await;

    tracing::info!("First-run wizard finalized by user {}", auth.user_id);

    Ok(Json(FinalizeResponse {
        wizard_completed: true,
        message: "Setup wizard completed.".into(),
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
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await?;

    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed. Use the normal registration endpoint.".into(),
        ));
    }

    // ── Validate username ───────────────────────────────────────────────────
    validation::validate_username(&req.username)?;

    // ── Validate password ───────────────────────────────────────────────────
    validation::validate_password(&req.password)?;

    // ── Create user ─────────────────────────────────────────────────────────
    let user_id = Uuid::new_v4().to_string();
    let password_clone = req.password.clone();
    let cost = state.config.auth.bcrypt_cost;
    let password_hash = tokio::task::spawn_blocking(move || bcrypt::hash(&password_clone, cost))
        .await
        .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?
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
        &state,
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
