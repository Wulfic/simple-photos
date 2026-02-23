//! Storage and directory browsing admin endpoints.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::admin::require_admin;

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
pub fn update_config_toml_storage(new_root: &str) -> anyhow::Result<()> {
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

// ── Directory Browser ───────────────────────────────────────────────────────

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
