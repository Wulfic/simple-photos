use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::state::AppState;

use super::models::*;
// ── Plain Photo Endpoints ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PhotoListQuery {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub media_type: Option<String>,
}

/// GET /api/photos
/// List plain-mode photos for the authenticated user.
pub async fn list_photos(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<PhotoListQuery>,
) -> Result<Json<PhotoListResponse>, AppError> {
    let limit = params.limit.unwrap_or(100).min(500);

    let photos = if let Some(ref after) = params.after {
        if let Some(ref mt) = params.media_type {
            sqlx::query_as::<_, PhotoRecord>(
                "SELECT id, filename, file_path, mime_type, media_type, size_bytes, width, height, \
                 duration_secs, taken_at, latitude, longitude, thumb_path, created_at \
                 FROM photos WHERE user_id = ? AND media_type = ? AND created_at > ? \
                 AND encrypted_blob_id IS NULL \
                 ORDER BY created_at ASC LIMIT ?",
            )
            .bind(&auth.user_id)
            .bind(mt)
            .bind(after)
            .bind(limit + 1)
            .fetch_all(&state.pool)
            .await?
        } else {
            sqlx::query_as::<_, PhotoRecord>(
                "SELECT id, filename, file_path, mime_type, media_type, size_bytes, width, height, \
                 duration_secs, taken_at, latitude, longitude, thumb_path, created_at \
                 FROM photos WHERE user_id = ? AND created_at > ? \
                 AND encrypted_blob_id IS NULL \
                 ORDER BY created_at ASC LIMIT ?",
            )
            .bind(&auth.user_id)
            .bind(after)
            .bind(limit + 1)
            .fetch_all(&state.pool)
            .await?
        }
    } else if let Some(ref mt) = params.media_type {
        sqlx::query_as::<_, PhotoRecord>(
            "SELECT id, filename, file_path, mime_type, media_type, size_bytes, width, height, \
             duration_secs, taken_at, latitude, longitude, thumb_path, created_at \
             FROM photos WHERE user_id = ? AND media_type = ? \
             AND encrypted_blob_id IS NULL \
             ORDER BY created_at ASC LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(mt)
        .bind(limit + 1)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as::<_, PhotoRecord>(
            "SELECT id, filename, file_path, mime_type, media_type, size_bytes, width, height, \
             duration_secs, taken_at, latitude, longitude, thumb_path, created_at \
             FROM photos WHERE user_id = ? \
             AND encrypted_blob_id IS NULL \
             ORDER BY created_at ASC LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(limit + 1)
        .fetch_all(&state.pool)
        .await?
    };

    let next_cursor = if photos.len() as i64 > limit {
        photos.last().map(|p| p.created_at.clone())
    } else {
        None
    };

    let photos: Vec<PhotoRecord> = photos.into_iter().take(limit as usize).collect();

    Ok(Json(PhotoListResponse {
        photos,
        next_cursor,
    }))
}

/// POST /api/photos/register
/// Register a plain file on disk as a photo in the database.
/// The file must already exist at the given path within the storage root.
pub async fn register_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<RegisterPhotoRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Security: ensure file_path doesn't escape storage root
    if req.file_path.contains("..") {
        return Err(AppError::BadRequest(
            "file_path must not contain '..'".into(),
        ));
    }

    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&req.file_path);

    // Verify the file actually exists
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err(AppError::BadRequest(format!(
            "File does not exist: {}",
            req.file_path
        )));
    }

    let photo_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let media_type = req.media_type.unwrap_or_else(|| {
        if req.mime_type.starts_with("video/") {
            "video".to_string()
        } else if req.mime_type == "image/gif" {
            "gif".to_string()
        } else {
            "photo".to_string()
        }
    });

    // Generate thumbnail path (will be created by a separate endpoint/process)
    let thumb_filename = format!("{}.thumb.jpg", photo_id);
    let thumb_rel = format!(".thumbnails/{}", thumb_filename);

    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&req.filename)
    .bind(&req.file_path)
    .bind(&req.mime_type)
    .bind(&media_type)
    .bind(req.size_bytes)
    .bind(req.width.unwrap_or(0))
    .bind(req.height.unwrap_or(0))
    .bind(req.duration_secs)
    .bind(&req.taken_at)
    .bind(req.latitude)
    .bind(req.longitude)
    .bind(&thumb_rel)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "photo_id": photo_id,
            "thumb_path": thumb_rel,
        })),
    ))
}

/// GET /api/photos/:id/file
/// Serve the original (unencrypted) photo file from disk.
pub async fn serve_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    let (file_path, mime_type, size_bytes): (String, String, i64) = sqlx::query_as(
        "SELECT file_path, mime_type, size_bytes FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| {
        tracing::warn!(
            user_id = %auth.user_id,
            photo_id = %photo_id,
            "serve_photo: photo not found in database"
        );
        AppError::NotFound
    })?;

    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&file_path);

    tracing::debug!(
        user_id = %auth.user_id,
        photo_id = %photo_id,
        file_path = %file_path,
        full_path = %full_path.display(),
        size_bytes = size_bytes,
        "serve_photo: serving file"
    );

    let file = tokio::fs::File::open(&full_path).await.map_err(|e| {
        tracing::error!(
            user_id = %auth.user_id,
            photo_id = %photo_id,
            file_path = %file_path,
            full_path = %full_path.display(),
            error = %e,
            "serve_photo: failed to open file on disk"
        );
        match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open photo: {}", e)),
        }
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            "Content-Type",
            HeaderValue::from_str(&mime_type).unwrap_or(HeaderValue::from_static("application/octet-stream")),
        )
        .header("Content-Length", HeaderValue::from(size_bytes))
        .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"))
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// GET /api/photos/:id/thumb
/// Serve the thumbnail for a plain-mode photo.
pub async fn serve_thumbnail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    let thumb_path: Option<String> = sqlx::query_scalar(
        "SELECT thumb_path FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;
    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&thumb_path);

    // If thumbnail doesn't exist yet, try to generate it on-the-fly
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        // Fall back to serving the original photo (client can resize)
        let (file_path, mime_type): (String, String) = sqlx::query_as(
            "SELECT file_path, mime_type FROM photos WHERE id = ? AND user_id = ?",
        )
        .bind(&photo_id)
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;

        let orig_path = storage_root.join(&file_path);
        let file = tokio::fs::File::open(&orig_path).await.map_err(|e| {
            AppError::Internal(format!("Failed to open photo for thumbnail: {}", e))
        })?;

        let stream = tokio_util::io::ReaderStream::new(file);
        let body = Body::from_stream(stream);

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(
                "Content-Type",
                HeaderValue::from_str(&mime_type)
                    .unwrap_or(HeaderValue::from_static("image/jpeg")),
            )
            .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"))
            .body(body)
            .map_err(|e| AppError::Internal(e.to_string()))?);
    }

    let meta = tokio::fs::metadata(&full_path).await.map_err(|e| {
        AppError::Internal(format!("Failed to read thumbnail: {}", e))
    })?;
    let file = tokio::fs::File::open(&full_path).await.map_err(|e| {
        AppError::Internal(format!("Failed to open thumbnail: {}", e))
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_static("image/jpeg"))
        .header("Content-Length", HeaderValue::from(meta.len()))
        .header("Cache-Control", HeaderValue::from_static("private, max-age=86400"))
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

// DELETE /api/photos/:id is now handled by trash::handlers::soft_delete_photo
// Photos are soft-deleted to the trash with a 30-day retention period.

/// POST /api/admin/photos/scan
/// Scan the storage directory and register all unregistered media files as plain photos.
/// This is the main "import" mechanism for plain mode.
pub async fn scan_and_register(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify admin
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let storage_root = state.storage_root.read().await.clone();

    // Get already-registered file paths
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT file_path FROM photos WHERE user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;
    let existing_set: std::collections::HashSet<String> = existing.into_iter().collect();

    // Scan recursively for media files
    let mut new_count = 0i64;
    let mut queue = vec![storage_root.clone()];

    while let Some(dir) = queue.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() && is_media_file(&name) {
                    let abs_path = entry.path();
                    let rel_path = abs_path
                        .strip_prefix(&storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .to_string();

                    if existing_set.contains(&rel_path) {
                        continue; // Already registered
                    }

                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            dt.to_rfc3339()
                        })
                    });

                    let mime = mime_from_extension(&name).to_string();
                    let media_type = if mime.starts_with("video/") {
                        "video"
                    } else if mime == "image/gif" {
                        "gif"
                    } else {
                        "photo"
                    };

                    let photo_id = Uuid::new_v4().to_string();
                    let now = Utc::now().to_rfc3339();
                    let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

                    sqlx::query(
                        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                         size_bytes, width, height, taken_at, thumb_path, created_at) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?)",
                    )
                    .bind(&photo_id)
                    .bind(&auth.user_id)
                    .bind(&name)
                    .bind(&rel_path)
                    .bind(&mime)
                    .bind(media_type)
                    .bind(size)
                    .bind(&modified)
                    .bind(&thumb_rel)
                    .bind(&now)
                    .execute(&state.pool)
                    .await?;

                    new_count += 1;
                }
            }
        }
    }

    tracing::info!("Scan complete: registered {} new photos", new_count);

    Ok(Json(serde_json::json!({
        "registered": new_count,
        "message": format!("{} new photos registered", new_count),
    })))
}
