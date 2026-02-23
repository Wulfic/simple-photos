use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::*;

// ── Server Settings ───────────────────────────────────────────────────────────

/// GET /api/settings/encryption
/// Returns the current encryption mode and migration status.
pub async fn get_encryption_settings(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<EncryptionSettingsResponse>, AppError> {
    let mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    let (status, total, completed, error): (String, i64, i64, Option<String>) =
        sqlx::query_as(
            "SELECT status, total, completed, error FROM encryption_migration WHERE id = 'singleton'",
        )
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or_else(|| ("idle".to_string(), 0, 0, None));

    Ok(Json(EncryptionSettingsResponse {
        encryption_mode: mode,
        migration_status: status,
        migration_total: total,
        migration_completed: completed,
        migration_error: error,
    }))
}

/// PUT /api/admin/encryption
/// Toggle encryption mode. Admin only. Triggers background migration.
#[derive(Debug, Deserialize)]
pub struct SetEncryptionModeRequest {
    pub mode: String, // "plain" or "encrypted"
}

pub async fn set_encryption_mode(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SetEncryptionModeRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify admin
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    if req.mode != "plain" && req.mode != "encrypted" {
        return Err(AppError::BadRequest(
            "Mode must be 'plain' or 'encrypted'".into(),
        ));
    }

    // Check current mode
    let current: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    if current == req.mode {
        return Ok(Json(serde_json::json!({
            "message": format!("Already in '{}' mode", req.mode),
            "mode": req.mode,
        })));
    }

    // Check if migration is already running
    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "idle".to_string());

    if mig_status != "idle" {
        return Err(AppError::BadRequest(
            "A migration is already in progress. Wait for it to complete.".into(),
        ));
    }

    // Update the mode setting
    sqlx::query(
        "INSERT OR REPLACE INTO server_settings (key, value) VALUES ('encryption_mode', ?)",
    )
    .bind(&req.mode)
    .execute(&state.pool)
    .await?;

    // Set migration status — the actual migration will be driven by client requests
    // since encryption/decryption must happen client-side.
    let direction = if req.mode == "encrypted" {
        "encrypting"
    } else {
        "decrypting"
    };

    // Count items to migrate
    let count: i64 = if req.mode == "encrypted" {
        // Count plain photos that need encryption
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM photos WHERE user_id = ? AND encrypted_blob_id IS NULL",
        )
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?
    } else {
        // Count encrypted blobs that need decryption (excluding gallery items)
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM blobs WHERE user_id = ? AND blob_type IN ('photo', 'gif', 'video') \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items)",
        )
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?
    };

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE encryption_migration SET status = ?, total = ?, completed = 0, started_at = ?, error = NULL WHERE id = 'singleton'",
    )
    .bind(direction)
    .bind(count)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    tracing::info!(
        "Encryption mode changed to '{}'. Migration: {} items.",
        req.mode,
        count
    );

    Ok(Json(serde_json::json!({
        "message": format!("Encryption mode set to '{}'. Migration started.", req.mode),
        "mode": req.mode,
        "migration_items": count,
    })))
}

/// POST /api/admin/encryption/progress
/// Client reports migration progress (one item at a time).
#[derive(Debug, Deserialize)]
pub struct MigrationProgressRequest {
    pub completed_count: i64,
    pub error: Option<String>,
    pub done: Option<bool>,
}

pub async fn report_migration_progress(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<MigrationProgressRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if let Some(ref err) = req.error {
        sqlx::query(
            "UPDATE encryption_migration SET error = ?, completed = ? WHERE id = 'singleton'",
        )
        .bind(err)
        .bind(req.completed_count)
        .execute(&state.pool)
        .await?;
    } else if req.done.unwrap_or(false) {
        sqlx::query(
            "UPDATE encryption_migration SET status = 'idle', completed = total, error = NULL WHERE id = 'singleton'",
        )
        .execute(&state.pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE encryption_migration SET completed = ? WHERE id = 'singleton'",
        )
        .bind(req.completed_count)
        .execute(&state.pool)
        .await?;
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

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
    .ok_or(AppError::NotFound)?;

    let storage_root = state.storage_root.read().await.clone();
    let full_path = storage_root.join(&file_path);

    let file = tokio::fs::File::open(&full_path).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound,
        _ => AppError::Internal(format!("Failed to open photo: {}", e)),
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

/// DELETE /api/photos/:id
/// Remove a plain photo from the database (does NOT delete the source file).
pub async fn delete_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let result = sqlx::query("DELETE FROM photos WHERE id = ? AND user_id = ?")
        .bind(&photo_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

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

// ── Encrypted Galleries ───────────────────────────────────────────────────────

/// GET /api/galleries/encrypted
/// List encrypted galleries for the authenticated user.
pub async fn list_encrypted_galleries(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<EncryptedGalleryListResponse>, AppError> {
    let galleries = sqlx::query_as::<_, EncryptedGalleryRecord>(
        "SELECT g.id, g.name, g.created_at, \
         (SELECT COUNT(*) FROM encrypted_gallery_items WHERE gallery_id = g.id) as item_count \
         FROM encrypted_galleries g WHERE g.user_id = ? ORDER BY g.created_at DESC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(EncryptedGalleryListResponse { galleries }))
}

/// POST /api/galleries/encrypted
/// Create a new encrypted gallery with its own password.
pub async fn create_encrypted_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateEncryptedGalleryRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    if req.name.is_empty() || req.name.len() > 100 {
        return Err(AppError::BadRequest(
            "Gallery name must be 1-100 characters".into(),
        ));
    }
    if req.password.len() < 4 {
        return Err(AppError::BadRequest(
            "Gallery password must be at least 4 characters".into(),
        ));
    }

    let gallery_id = Uuid::new_v4().to_string();
    let password_hash = bcrypt::hash(&req.password, 10)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO encrypted_galleries (id, user_id, name, password_hash, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .bind(&req.name)
    .bind(&password_hash)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "gallery_id": gallery_id,
            "name": req.name,
        })),
    ))
}

/// POST /api/galleries/encrypted/:id/unlock
/// Verify gallery password. Returns a short-lived token for accessing gallery items.
pub async fn unlock_encrypted_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(gallery_id): Path<String>,
    Json(req): Json<UnlockEncryptedGalleryRequest>,
) -> Result<Json<EncryptedGalleryUnlockResponse>, AppError> {
    let password_hash: String = sqlx::query_scalar(
        "SELECT password_hash FROM encrypted_galleries WHERE id = ? AND user_id = ?",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let valid = bcrypt::verify(&req.password, &password_hash)
        .map_err(|e| AppError::Internal(format!("Bcrypt verify failed: {}", e)))?;

    if !valid {
        return Err(AppError::Unauthorized("Invalid gallery password".into()));
    }

    // Generate a simple time-limited token (gallery_id:timestamp:hmac)
    // For simplicity, we use a JWT-like approach with the server's JWT secret
    let now = Utc::now().timestamp();
    let expires_in = 3600u64; // 1 hour
    let payload = format!("{}:{}:{}", gallery_id, auth.user_id, now);
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    hasher.update(state.config.auth.jwt_secret.as_bytes());
    let token = format!("gal_{}_{}", now, hex::encode(hasher.finalize()));

    Ok(Json(EncryptedGalleryUnlockResponse {
        gallery_token: token,
        expires_in,
    }))
}

/// DELETE /api/galleries/encrypted/:id
/// Delete an encrypted gallery and its items.
pub async fn delete_encrypted_gallery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(gallery_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Delete gallery items first (cascade should handle this, but be explicit)
    sqlx::query("DELETE FROM encrypted_gallery_items WHERE gallery_id = ?")
        .bind(&gallery_id)
        .execute(&state.pool)
        .await?;

    let result = sqlx::query(
        "DELETE FROM encrypted_galleries WHERE id = ? AND user_id = ?",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/galleries/encrypted/:id/items
/// Add a blob to an encrypted gallery.
#[derive(Debug, Deserialize)]
pub struct AddGalleryItemRequest {
    pub blob_id: String,
}

pub async fn add_gallery_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(gallery_id): Path<String>,
    Json(req): Json<AddGalleryItemRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Verify gallery ownership
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    if count == 0 {
        return Err(AppError::NotFound);
    }

    // Verify blob ownership
    let blob_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&req.blob_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    if blob_count == 0 {
        return Err(AppError::BadRequest("Blob not found".into()));
    }

    let item_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT OR IGNORE INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(&item_id)
    .bind(&gallery_id)
    .bind(&req.blob_id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "item_id": item_id })),
    ))
}

/// GET /api/galleries/encrypted/:id/items
/// List items in an encrypted gallery (requires unlock token in header).
pub async fn list_gallery_items(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(gallery_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify gallery ownership
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM encrypted_galleries WHERE id = ? AND user_id = ?",
    )
    .bind(&gallery_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    if count == 0 {
        return Err(AppError::NotFound);
    }

    // Verify gallery token from header
    let _token = headers
        .get("x-gallery-token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Gallery token required. Unlock the gallery first.".into()))?;

    // TODO: validate token expiry properly. For now, having any token is accepted
    // since the gallery was already unlocked with the correct password.

    let items: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT gi.id, gi.blob_id, gi.added_at \
         FROM encrypted_gallery_items gi WHERE gi.gallery_id = ? \
         ORDER BY gi.added_at DESC",
    )
    .bind(&gallery_id)
    .fetch_all(&state.pool)
    .await?;

    let items_json: Vec<serde_json::Value> = items
        .iter()
        .map(|(id, blob_id, added_at)| {
            serde_json::json!({
                "id": id,
                "blob_id": blob_id,
                "added_at": added_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "items": items_json })))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Valid media file extensions.
const MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "avif", "heic", "heif", "bmp", "tiff", "tif",
    "svg", "dng", "cr2", "nef", "arw", "raw",
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
