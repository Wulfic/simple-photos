//! Server-side import: scan directories and stream files for client-side upload.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::state::AppState;

use super::admin::require_admin;

/// Query parameters for scanning a server-side directory for importable media files.
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
/// Returns all image/video files (recursive scan through subdirectories).
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

    // Recursively scan directories for media files
    let mut queue = vec![canonical.clone()];

    while let Some(dir) = queue.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await.map_err(|e| {
            AppError::BadRequest(format!("Cannot read directory '{}': {}", dir.display(), e))
        })?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() && is_media_file(&name) {
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
    }

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

/// Query parameters for streaming a single file from the server filesystem for client-side import.
#[derive(Debug, Deserialize)]
pub struct ImportFileQuery {
    pub path: String,
}

/// Admin-only: Stream a raw file from the server filesystem for client-side import.
///
/// GET /api/admin/import/file?path=/path/to/photo.jpg
///
/// The client downloads the raw file, encrypts it locally, then uploads as a blob.
pub async fn import_file(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<ImportFileQuery>,
) -> Result<axum::response::Response, AppError> {
    require_admin(&state, &auth).await?;

    let file_path = PathBuf::from(&query.path);

    if query.path.contains("..") {
        return Err(AppError::BadRequest("Path must not contain '..'".into()));
    }

    let canonical = tokio::fs::canonicalize(&file_path).await.map_err(|e| {
        AppError::BadRequest(format!("Cannot resolve path '{}': {}", query.path, e))
    })?;

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
