//! First-run setup wizard API endpoints.
//!
//! These endpoints are used by the web frontend's setup wizard to bootstrap
//! the application on first run. They allow creating the initial admin user
//! without requiring authentication (since no users exist yet).
//!
//! Also includes admin-only endpoints for creating additional users and
//! updating server configuration during setup.
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
use crate::error::AppError;
use crate::state::AppState;
use std::path::PathBuf;

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

// ── Admin-only endpoints ────────────────────────────────────────────────────

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

/// Helper to check if the requesting user has admin role.
async fn require_admin(state: &AppState, auth: &AuthUser) -> Result<(), AppError> {
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

#[derive(Debug, Serialize)]
pub struct StorageResponse {
    pub storage_path: String,
    pub message: String,
}

/// Admin-only: Get the current storage path.
///
/// GET /api/admin/storage
pub async fn get_storage(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<StorageResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let root = state.storage_root.read().await;
    let path = root.display().to_string();
    Ok(Json(StorageResponse {
        storage_path: path,
        message: "Current storage path".into(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateStorageRequest {
    pub path: String,
}

/// Admin-only: Update the storage root directory.
///
/// PUT /api/admin/storage
///
/// Validates the path exists (or creates it), updates in-memory config,
/// and writes the change back to config.toml.
pub async fn update_storage(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<UpdateStorageRequest>,
) -> Result<Json<StorageResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let new_path = PathBuf::from(&req.path);

    // Security: reject paths that try to escape (e.g. contain "..")
    if req.path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    // Create directory if it doesn't exist
    if !new_path.exists() {
        tokio::fs::create_dir_all(&new_path).await.map_err(|e| {
            AppError::BadRequest(format!(
                "Cannot create directory '{}': {}",
                new_path.display(),
                e
            ))
        })?;
    }

    // Verify it's actually a directory
    let meta = tokio::fs::metadata(&new_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot access '{}': {}", new_path.display(), e))
    })?;
    if !meta.is_dir() {
        return Err(AppError::BadRequest(format!(
            "'{}' is not a directory",
            new_path.display()
        )));
    }

    // Test write permissions by creating and removing a temp file
    let test_file = new_path.join(".simple-photos-write-test");
    tokio::fs::write(&test_file, b"test")
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "Directory '{}' is not writable: {}",
                new_path.display(),
                e
            ))
        })?;
    let _ = tokio::fs::remove_file(&test_file).await;

    // Update in-memory storage root
    {
        let mut root = state.storage_root.write().await;
        *root = new_path.clone();
    }

    // Persist to config.toml
    if let Err(e) = update_config_toml_storage(&req.path) {
        tracing::warn!("Failed to persist storage path to config.toml: {}", e);
    }

    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "update_storage_path",
            "new_path": req.path,
        })),
    )
    .await;

    tracing::info!("Storage path updated to: {}", req.path);

    Ok(Json(StorageResponse {
        storage_path: req.path,
        message: "Storage path updated successfully".into(),
    }))
}

/// Read config.toml, update [storage] root, and write it back.
fn update_config_toml_storage(new_root: &str) -> anyhow::Result<()> {
    let config_path = std::env::var("SIMPLE_PHOTOS_CONFIG")
        .unwrap_or_else(|_| "config.toml".into());
    let contents = std::fs::read_to_string(&config_path)?;
    let mut doc: toml::Table = contents.parse()?;

    if let Some(storage) = doc.get_mut("storage").and_then(|v| v.as_table_mut()) {
        storage.insert("root".to_string(), toml::Value::String(new_root.to_string()));
    }

    std::fs::write(&config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BrowseResponse {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub directories: Vec<DirEntry>,
    pub writable: bool,
}

#[derive(Debug, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
}

/// Admin-only: Browse server filesystem directories.
///
/// GET /api/admin/browse?path=/some/path
///
/// Returns subdirectories only (no files) for the file browser UI.
/// Defaults to the current storage root if no path is given.
pub async fn browse_directory(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<BrowseQuery>,
) -> Result<Json<BrowseResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let browse_path = match &query.path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => state.storage_root.read().await.clone(),
    };

    // Security: reject ".." traversal
    let path_str = browse_path.display().to_string();
    if path_str.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    // Canonicalize to get absolute path
    let canonical = tokio::fs::canonicalize(&browse_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot resolve path '{}': {}", browse_path.display(), e))
    })?;

    let meta = tokio::fs::metadata(&canonical).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot access '{}': {}", canonical.display(), e))
    })?;

    if !meta.is_dir() {
        return Err(AppError::BadRequest(format!(
            "'{}' is not a directory",
            canonical.display()
        )));
    }

    // Read directory entries — directories only, skip hidden ones
    let mut directories = Vec::new();
    let mut entries = tokio::fs::read_dir(&canonical).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot read directory '{}': {}", canonical.display(), e))
    })?;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if let Ok(ft) = entry.file_type().await {
            if ft.is_dir() {
                let full_path = entry.path().display().to_string();
                directories.push(DirEntry {
                    name,
                    path: full_path,
                });
            }
        }
    }

    directories.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    // Check if writable
    let test_file = canonical.join(".simple-photos-write-test");
    let writable = tokio::fs::write(&test_file, b"test").await.is_ok();
    if writable {
        let _ = tokio::fs::remove_file(&test_file).await;
    }

    let parent_path = canonical.parent().map(|p| p.display().to_string());

    Ok(Json(BrowseResponse {
        current_path: canonical.display().to_string(),
        parent_path,
        directories,
        writable,
    }))
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

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub role: String,
    pub totp_enabled: bool,
    pub created_at: String,
}
