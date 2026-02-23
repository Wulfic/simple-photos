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

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
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

/// Read config.toml, update [server] port (and base_url), and write it back.
fn update_config_toml_port(new_port: u16) -> anyhow::Result<()> {
    let config_path = std::env::var("SIMPLE_PHOTOS_CONFIG")
        .unwrap_or_else(|_| "config.toml".into());
    let contents = std::fs::read_to_string(&config_path)?;
    let mut doc: toml::Table = contents.parse()?;

    if let Some(server) = doc.get_mut("server").and_then(|v| v.as_table_mut()) {
        server.insert("port".to_string(), toml::Value::Integer(new_port as i64));

        // Also update base_url to reflect the new port.
        // Replace the port portion of URLs like "http://localhost:8080" or "http://host:1234"
        if let Some(base_url) = server.get("base_url").and_then(|v| v.as_str()) {
            let updated = if let Some(colon_pos) = base_url.rfind(':') {
                // Check that after the colon we have digits (port), possibly followed by a path
                let after_colon = &base_url[colon_pos + 1..];
                let port_end = after_colon.find('/').unwrap_or(after_colon.len());
                if after_colon[..port_end].parse::<u16>().is_ok() {
                    format!(
                        "{}:{}{}",
                        &base_url[..colon_pos],
                        new_port,
                        &after_colon[port_end..]
                    )
                } else {
                    base_url.to_string()
                }
            } else {
                base_url.to_string()
            };
            server.insert("base_url".to_string(), toml::Value::String(updated));
        }
    }

    std::fs::write(&config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

// ── Port configuration endpoints ────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PortResponse {
    pub port: u16,
    pub message: String,
}

/// Admin-only: Get the current server port from config.
///
/// GET /api/admin/port
pub async fn get_port(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<PortResponse>, AppError> {
    require_admin(&state, &auth).await?;

    Ok(Json(PortResponse {
        port: state.config.server.port,
        message: "Current server port".into(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePortRequest {
    pub port: u16,
}

/// Admin-only: Update the server port in config.toml.
///
/// PUT /api/admin/port
///
/// This only persists the change to config.toml. The server must be
/// restarted for the new port to take effect.
pub async fn update_port(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<UpdatePortRequest>,
) -> Result<Json<PortResponse>, AppError> {
    require_admin(&state, &auth).await?;

    // Validate port range (1024–65535 for non-privileged ports)
    if req.port < 1024 {
        return Err(AppError::BadRequest(
            "Port must be 1024 or higher (non-privileged range)".into(),
        ));
    }

    // Persist to config.toml
    update_config_toml_port(req.port).map_err(|e| {
        tracing::error!("Failed to update port in config.toml: {}", e);
        AppError::Internal(format!("Failed to save port configuration: {}", e))
    })?;

    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "update_port",
            "new_port": req.port,
        })),
    )
    .await;

    tracing::info!("Server port updated to {} in config (restart required)", req.port);

    Ok(Json(PortResponse {
        port: req.port,
        message: format!(
            "Port updated to {}. Server restart required for the change to take effect.",
            req.port
        ),
    }))
}

// ── Server restart endpoint ─────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RestartResponse {
    pub message: String,
}

/// Admin-only: Trigger a graceful server restart.
///
/// POST /api/admin/restart
///
/// The server exits after a short delay (to allow the HTTP response to be sent).
/// A service manager (systemd, Docker, etc.) or the user is expected to restart
/// the process, which will pick up any config.toml changes (e.g. new port).
pub async fn restart_server(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
) -> Result<Json<RestartResponse>, AppError> {
    require_admin(&state, &auth).await?;

    audit::log(
        &state.pool,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({ "action": "server_restart" })),
    )
    .await;

    tracing::info!("Server restart requested by admin — shutting down in 1 second");

    // Spawn a task that exits after a brief delay so the HTTP response is sent first
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        tracing::info!("Exiting for restart…");
        std::process::exit(0);
    });

    Ok(Json(RestartResponse {
        message: "Server is restarting. Please wait…".into(),
    }))
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

// ── Server-side Import: Scan & Serve ──────────────────────────────────────────

/// Valid media file extensions for server-side scanning.
const MEDIA_EXTENSIONS: &[&str] = &[
    // Images
    "jpg", "jpeg", "png", "gif", "webp", "avif", "heic", "heif", "bmp", "tiff", "tif",
    "svg", "dng", "cr2", "nef", "arw", "raw",
    // Videos
    "mp4", "mov", "mkv", "webm", "avi", "3gp", "m4v",
];

fn is_media_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    MEDIA_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{}", ext)))
}

fn mime_from_extension(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "3gp" => "video/3gpp",
        "m4v" => "video/x-m4v",
        _ => "application/octet-stream",
    }
}

#[derive(Debug, Deserialize)]
pub struct ImportScanQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportScanResponse {
    pub directory: String,
    pub files: Vec<MediaFileEntry>,
    pub total_size: u64,
}

#[derive(Debug, Serialize)]
pub struct MediaFileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub mime_type: String,
    pub modified: Option<String>,
}

/// Admin-only: Scan a directory for importable media files.
///
/// GET /api/admin/import/scan?path=/some/path
///
/// If no path is given, scans the current storage root.
/// Returns all image/video files (non-recursive — one level only).
pub async fn import_scan(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<ImportScanQuery>,
) -> Result<Json<ImportScanResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let scan_path = match &query.path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => state.storage_root.read().await.clone(),
    };

    let path_str = scan_path.display().to_string();
    if path_str.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let canonical = tokio::fs::canonicalize(&scan_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot resolve path '{}': {}", scan_path.display(), e))
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

    let mut files = Vec::new();
    let mut total_size: u64 = 0;

    // Scan directory entries — files only, skip hidden
    let mut entries = tokio::fs::read_dir(&canonical).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot read directory '{}': {}", canonical.display(), e))
    })?;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        if let Ok(ft) = entry.file_type().await {
            if ft.is_file() && is_media_file(&name) {
                let full_path = entry.path().display().to_string();
                let file_meta = entry.metadata().await.ok();
                let size = file_meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified = file_meta.and_then(|m| {
                    m.modified().ok().map(|t| {
                        let dt: chrono::DateTime<chrono::Utc> = t.into();
                        dt.to_rfc3339()
                    })
                });
                let mime = mime_from_extension(&name).to_string();

                total_size += size;
                files.push(MediaFileEntry {
                    name,
                    path: full_path,
                    size,
                    mime_type: mime,
                    modified,
                });
            }
        }
    }

    // Sort by name
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    tracing::info!(
        "Import scan: {} media files ({} bytes) in {:?}",
        files.len(),
        total_size,
        canonical
    );

    Ok(Json(ImportScanResponse {
        directory: canonical.display().to_string(),
        files,
        total_size,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ImportFileQuery {
    pub path: String,
}

/// Admin-only: Stream a raw file from the server filesystem for client-side import.
///
/// GET /api/admin/import/file?path=/path/to/photo.jpg
///
/// The client downloads the raw file, encrypts it locally, then uploads as a blob.
/// This enables importing photos already on the server without going through the browser file picker.
pub async fn import_file(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<ImportFileQuery>,
) -> Result<axum::response::Response, AppError> {
    require_admin(&state, &auth).await?;

    let file_path = PathBuf::from(&query.path);

    // Security checks
    if query.path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let canonical = tokio::fs::canonicalize(&file_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot resolve path '{}': {}", query.path, e))
    })?;

    // Verify it's a file and it's a media file
    let meta = tokio::fs::metadata(&canonical).await.map_err(|_e| {
        AppError::NotFound
    })?;

    if !meta.is_file() {
        return Err(AppError::BadRequest("Path is not a file".into()));
    }

    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if !is_media_file(&name) {
        return Err(AppError::BadRequest("Not a supported media file".into()));
    }

    let mime = mime_from_extension(&name);
    let size = meta.len();

    // Stream the file
    let file = tokio::fs::File::open(&canonical).await.map_err(|e| {
        AppError::Internal(format!("Failed to open file: {}", e))
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_static(mime))
        .header("Content-Length", HeaderValue::from(size))
        .header(
            "Content-Disposition",
            HeaderValue::from_str(&format!("inline; filename=\"{}\"", name)).unwrap_or_else(|_| {
                HeaderValue::from_static("inline")
            }),
        )
        .header("Cache-Control", HeaderValue::from_static("no-store"))
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}
