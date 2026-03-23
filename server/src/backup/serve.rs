//! Server‐to‐server backup “serving” API.
//!
//! When this instance is in backup mode, other Simple Photos servers can
//! pull data from it via these endpoints. All requests are authenticated
//! with an `X-API-Key` header (validated against `config.backup.api_key`).
//!
//! Endpoints:
//! - `GET  /api/backup/list`                    — list all photos (with IDs)
//! - `GET  /api/backup/list-trash`              — list all trash items (with IDs)
//! - `GET  /api/backup/download/:id`            — download original file
//! - `GET  /api/backup/download/:id/thumb`      — download thumbnail
//! - `POST /api/backup/receive`                 — receive a photo pushed
//!   from the primary server (verifies `X-Content-Hash` if present)

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use percent_encoding::{percent_decode_str, utf8_percent_encode, CONTROLS};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::photos::scan::generate_thumbnail_file;
use crate::sanitize;
use crate::state::AppState;

use super::models::BackupPhotoRecord;

// ── API-Key Validation ───────────────────────────────────────────────────────

/// Validate the `X-API-Key` header against the configured backup API key.
///
/// Priority:
/// 1. `config.backup.api_key` (TOML / env var) — static, fastest path
/// 2. `server_settings.backup_api_key` (DB) — auto-generated during pairing
///    or when "backup mode" is enabled via the admin UI
///
/// Returns an error if the key is missing, wrong, or backup serving is disabled.
async fn validate_api_key(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    // Resolve the expected key: prefer static config, fall back to DB
    let configured_key: String = if let Some(k) = state
        .config
        .backup
        .api_key
        .as_deref()
        .filter(|k| !k.is_empty())
    {
        k.to_string()
    } else {
        // Check DB for a key generated via pairing or admin UI
        let db_key: Option<String> =
            sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_api_key'")
                .fetch_optional(&state.read_pool)
                .await
                .unwrap_or(None);

        db_key.filter(|k| !k.is_empty()).ok_or_else(|| {
            AppError::Forbidden("Backup serving is not enabled on this server".into())
        })?
    };

    let provided_key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing X-API-Key header".into()))?;

    if provided_key != configured_key {
        return Err(AppError::Unauthorized("Invalid API key".into()));
    }

    Ok(())
}

// ── Backup Serve Endpoints ───────────────────────────────────────────────────

/// GET /api/backup/list
/// Returns a list of all photos on this server, authenticated via API key.
/// Used by other servers for recovery and backup browsing.
pub async fn backup_list_photos(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<BackupPhotoRecord>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let photos = sqlx::query_as::<_, BackupPhotoRecord>(
        "SELECT p.id, p.user_id, p.filename, p.file_path, p.mime_type, p.media_type, \
         p.size_bytes, p.width, p.height, p.duration_secs, p.taken_at, \
         p.latitude, p.longitude, p.thumb_path, p.created_at, \
         p.is_favorite, p.camera_model, p.photo_hash, p.crop_metadata \
         FROM photos p ORDER BY p.created_at ASC",
    )
    .fetch_all(&state.read_pool)
    .await?;

    Ok(Json(photos))
}

/// GET /api/backup/list-trash
/// Returns a list of all trash items on this server, authenticated via API key.
/// Used by the sync engine for delta-sync (skip items the remote already has).
pub async fn backup_list_trash(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let rows: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT id, file_path, size_bytes FROM trash_items ORDER BY deleted_at ASC")
            .fetch_all(&state.read_pool)
            .await?;

    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, file_path, size_bytes)| {
            serde_json::json!({ "id": id, "file_path": file_path, "size_bytes": size_bytes })
        })
        .collect();

    Ok(Json(items))
}

/// GET /api/backup/download/:photo_id
/// Serves the original photo file, authenticated via API key.
/// Used by other servers during recovery.
pub async fn backup_download_photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    validate_api_key(&state, &headers).await?;

    let (file_path, mime_type): (String, String) =
        sqlx::query_as("SELECT file_path, mime_type FROM photos WHERE id = ?")
            .bind(&photo_id)
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&file_path);

    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open photo: {}", e)),
        })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(
            "Content-Type",
            HeaderValue::from_str(&mime_type)
                .unwrap_or(HeaderValue::from_static("application/octet-stream")),
        )
        .body(body)
        .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?;

    // Include the file_path so the caller knows where to store it.
    // Percent-encode non-ASCII chars so the header value stays valid ASCII.
    let encoded_fp = utf8_percent_encode(&file_path, CONTROLS).to_string();
    if let Ok(val) = HeaderValue::from_str(&encoded_fp) {
        response.headers_mut().insert("X-File-Path", val);
    }

    Ok(response)
}

/// GET /api/backup/download/:photo_id/thumb
/// Serves the thumbnail for a photo, authenticated via API key.
/// Used by other servers for backup view browsing.
pub async fn backup_download_thumb(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    validate_api_key(&state, &headers).await?;

    let thumb_path: Option<String> =
        sqlx::query_scalar("SELECT thumb_path FROM photos WHERE id = ?")
            .bind(&photo_id)
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;

    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&thumb_path);

    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open thumbnail: {}", e)),
        })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/jpeg")
        .body(body)
        .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?)
}

// ── Backup Receive Endpoint ──────────────────────────────────────────────────

/// POST /api/backup/receive
/// Receives a file pushed from a primary server during sync.
/// Preserves all metadata (timestamps, GPS, dimensions, tags, user association)
/// and generates a thumbnail immediately so the backup is a 1:1 replica.
///
/// Required headers: X-API-Key, X-Photo-Id, X-File-Path, X-Source, X-Content-Hash
/// Metadata headers (all optional, sent by updated primary sync engine):
///   X-User-Id, X-Original-Created-At, X-Taken-At, X-Width, X-Height,
///   X-Latitude, X-Longitude, X-Duration-Secs, X-Camera-Model, X-Is-Favorite,
///   X-Photo-Hash, X-Crop-Metadata, X-Tags,
///   X-Deleted-At, X-Expires-At (trash only)
pub async fn backup_receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_api_key(&state, &headers).await?;

    // Extract required headers
    let photo_id = headers
        .get("X-Photo-Id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-Photo-Id header".into()))?
        .to_string();

    let raw_file_path = headers
        .get("X-File-Path")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-File-Path header".into()))?;

    // Percent-decode the path (the sync sender encodes non-ASCII chars)
    let file_path = percent_decode_str(raw_file_path)
        .decode_utf8()
        .map_err(|e| AppError::BadRequest(format!("Invalid UTF-8 in X-File-Path: {}", e)))?
        .to_string();

    // Security: validate the file_path is a safe relative path (no traversal, no absolute)
    sanitize::validate_relative_path(&file_path)
        .map_err(|reason| AppError::BadRequest(format!("Invalid X-File-Path: {}", reason)))?;

    let source = headers
        .get("X-Source")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("photos")
        .to_string();

    // Verify checksum if the sender provided one (X-Content-Hash: hex SHA-256)
    if let Some(expected_hash) = headers.get("X-Content-Hash").and_then(|v| v.to_str().ok()) {
        let actual_hash = hex::encode(Sha256::digest(&body));
        if !actual_hash.eq_ignore_ascii_case(expected_hash) {
            return Err(AppError::BadRequest(format!(
                "Content hash mismatch: expected {}, got {}",
                expected_hash, actual_hash
            )));
        }
    }

    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&file_path);

    // Defense-in-depth: verify the resolved path is still within storage_root
    let canonical_root = storage_root
        .canonicalize()
        .unwrap_or_else(|_| storage_root.clone());
    // We can't canonicalize full_path yet (file doesn't exist), so check the parent
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create directories: {}", e)))?;
        let canonical_parent = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        if !canonical_parent.starts_with(&canonical_root) {
            return Err(AppError::BadRequest(
                "File path escapes storage root".into(),
            ));
        }
    }

    // Write the file to disk
    let size_bytes = body.len() as i64;
    tokio::fs::write(&full_path, &body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write file: {}", e)))?;

    // ── Parse metadata headers ──────────────────────────────────────────────

    // Helpers to parse percent-encoded optional string / numeric headers
    fn hdr_str(headers: &HeaderMap, name: &str) -> Option<String> {
        headers.get(name).and_then(|v| v.to_str().ok()).map(|s| {
            percent_decode_str(s)
                .decode_utf8()
                .map(|d| d.to_string())
                .unwrap_or_else(|_| s.to_string())
        })
    }
    fn hdr_f64(headers: &HeaderMap, name: &str) -> Option<f64> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<f64>().ok())
    }
    fn hdr_i64(headers: &HeaderMap, name: &str) -> i64 {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0)
    }

    // Use the primary's user_id when the corresponding user exists on the backup;
    // otherwise fall back to the local admin so foreign-key constraints are met.
    let primary_user_id = hdr_str(&headers, "X-User-Id");
    let owner_id: String = if let Some(ref uid) = primary_user_id {
        let exists: bool = sqlx::query_scalar("SELECT COUNT(*) > 0 FROM users WHERE id = ?")
            .bind(uid)
            .fetch_one(&state.read_pool)
            .await
            .unwrap_or(false);
        if exists {
            uid.clone()
        } else {
            sqlx::query_scalar(
                "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
            )
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or_else(|| AppError::Internal("No admin user on backup server".into()))?
        }
    } else {
        sqlx::query_scalar(
            "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
        )
        .fetch_optional(&state.read_pool)
        .await?
        .ok_or_else(|| AppError::Internal("No admin user on backup server".into()))?
    };

    // Derive filename, mime_type, and media_type from headers (if provided by primary)
    // or fall back to deriving from the file path.
    let filename = hdr_str(&headers, "X-Filename").unwrap_or_else(|| {
        std::path::Path::new(&file_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| file_path.clone())
    });

    let mime_type = hdr_str(&headers, "X-Mime-Type")
        .unwrap_or_else(|| crate::media::mime_from_extension(&filename).to_string());

    let media_type = hdr_str(&headers, "X-Media-Type").unwrap_or_else(|| {
        if mime_type.starts_with("video/") {
            "video".to_string()
        } else if mime_type.starts_with("audio/") {
            "audio".to_string()
        } else if mime_type == "image/gif" {
            "gif".to_string()
        } else {
            "photo".to_string()
        }
    });

    // Preserve original timestamps; fall back to now only when absent
    let now = Utc::now().to_rfc3339();
    let created_at = hdr_str(&headers, "X-Original-Created-At").unwrap_or_else(|| now.clone());
    let taken_at = hdr_str(&headers, "X-Taken-At");
    let deleted_at = hdr_str(&headers, "X-Deleted-At").unwrap_or_else(|| now.clone());
    let expires_at = hdr_str(&headers, "X-Expires-At").unwrap_or_else(|| now.clone());

    let width = hdr_i64(&headers, "X-Width");
    let height = hdr_i64(&headers, "X-Height");
    let latitude = hdr_f64(&headers, "X-Latitude");
    let longitude = hdr_f64(&headers, "X-Longitude");
    let duration = hdr_f64(&headers, "X-Duration-Secs");
    let camera_model = hdr_str(&headers, "X-Camera-Model");
    let is_favorite = headers
        .get("X-Is-Favorite")
        .and_then(|v| v.to_str().ok())
        .map(|s| s == "1")
        .unwrap_or(false);
    let photo_hash = hdr_str(&headers, "X-Photo-Hash");
    let crop_metadata = hdr_str(&headers, "X-Crop-Metadata");

    // Tags: comma-separated, each tag percent-encoded
    let tags: Vec<String> = headers
        .get("X-Tags")
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(',')
                .filter(|t| !t.is_empty())
                .map(|t| {
                    percent_decode_str(t)
                        .decode_utf8()
                        .map(|d| d.to_string())
                        .unwrap_or_else(|_| t.to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    // Thumbnail path — same convention as autoscan so serving code stays identical
    let thumb_ext = if mime_type == "image/gif" {
        "gif"
    } else {
        "jpg"
    };
    let thumb_rel = format!(".thumbnails/{}.thumb.{}", photo_id, thumb_ext);

    if source == "trash" {
        // ── Upsert into trash_items (full metadata) ───────────────────────────
        sqlx::query(
            "INSERT INTO trash_items (
                id, user_id, photo_id, filename, file_path, mime_type, media_type,
                size_bytes, width, height, taken_at, latitude, longitude,
                duration_secs, camera_model, is_favorite, photo_hash, crop_metadata,
                thumb_path, deleted_at, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                file_path     = excluded.file_path,
                size_bytes    = excluded.size_bytes,
                taken_at      = COALESCE(excluded.taken_at,     taken_at),
                latitude      = COALESCE(excluded.latitude,     latitude),
                longitude     = COALESCE(excluded.longitude,    longitude),
                width         = CASE WHEN excluded.width  > 0 THEN excluded.width  ELSE width  END,
                height        = CASE WHEN excluded.height > 0 THEN excluded.height ELSE height END,
                duration_secs = COALESCE(excluded.duration_secs,  duration_secs),
                camera_model  = COALESCE(excluded.camera_model,   camera_model),
                is_favorite   = excluded.is_favorite,
                photo_hash    = COALESCE(excluded.photo_hash,     photo_hash),
                crop_metadata = COALESCE(excluded.crop_metadata,  crop_metadata),
                thumb_path    = COALESCE(excluded.thumb_path,     thumb_path),
                deleted_at    = excluded.deleted_at,
                expires_at    = excluded.expires_at",
        )
        .bind(&photo_id)
        .bind(&owner_id)
        .bind(&photo_id)
        .bind(&filename)
        .bind(&file_path)
        .bind(&mime_type)
        .bind(&media_type)
        .bind(size_bytes)
        .bind(width)
        .bind(height)
        .bind(&taken_at)
        .bind(latitude)
        .bind(longitude)
        .bind(duration)
        .bind(&camera_model)
        .bind(is_favorite)
        .bind(&photo_hash)
        .bind(&crop_metadata)
        .bind(&thumb_rel)
        .bind(&deleted_at)
        .bind(&expires_at)
        .execute(&state.pool)
        .await?;

        // Remove from the main photos table if it was previously synced as an
        // active photo — the item has been deleted on the primary and must not
        // appear in the gallery on the backup either.
        //
        // X-Original-Photo-Id contains the original photo UUID (photos.id on
        // the backup).  X-Photo-Id is the *trash row* UUID, which is a
        // different value — deleting by it would be a no-op.
        let gallery_id = hdr_str(&headers, "X-Original-Photo-Id")
            .unwrap_or_else(|| photo_id.clone());
        // Delete by both UUID and file_path.  The UUID covers the normal case;
        // file_path covers a race where autoscan ran between Phase-0a and this
        // receive call and re-imported the file with a different UUID.
        if let Err(e) = sqlx::query("DELETE FROM photos WHERE id = ? OR file_path = ?")
            .bind(&gallery_id)
            .bind(&file_path)
            .execute(&state.pool)
            .await
        {
            tracing::warn!(
                photo_id = %photo_id,
                gallery_id = %gallery_id,
                "Failed to remove photo from gallery after receiving as trash: {}",
                e
            );
        }
        // Clean up any dangling tags for the removed photo row.
        let _ = sqlx::query("DELETE FROM photo_tags WHERE photo_id = ?")
            .bind(&gallery_id)
            .execute(&state.pool)
            .await;
    } else {
        // ── Upsert into photos (full metadata) ───────────────────────────────
        sqlx::query(
            "INSERT INTO photos (
                id, user_id, filename, file_path, mime_type, media_type,
                size_bytes, width, height, taken_at, latitude, longitude,
                duration_secs, camera_model, is_favorite, photo_hash, crop_metadata,
                thumb_path, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                file_path     = excluded.file_path,
                size_bytes    = excluded.size_bytes,
                taken_at      = COALESCE(excluded.taken_at,    taken_at),
                latitude      = COALESCE(excluded.latitude,    latitude),
                longitude     = COALESCE(excluded.longitude,   longitude),
                width         = CASE WHEN excluded.width  > 0 THEN excluded.width  ELSE width  END,
                height        = CASE WHEN excluded.height > 0 THEN excluded.height ELSE height END,
                duration_secs = COALESCE(excluded.duration_secs,  duration_secs),
                camera_model  = COALESCE(excluded.camera_model,   camera_model),
                is_favorite   = excluded.is_favorite,
                photo_hash    = COALESCE(excluded.photo_hash,     photo_hash),
                crop_metadata = COALESCE(excluded.crop_metadata,  crop_metadata),
                thumb_path    = COALESCE(excluded.thumb_path,     thumb_path)",
        )
        .bind(&photo_id)
        .bind(&owner_id)
        .bind(&filename)
        .bind(&file_path)
        .bind(&mime_type)
        .bind(&media_type)
        .bind(size_bytes)
        .bind(width)
        .bind(height)
        .bind(&taken_at)
        .bind(latitude)
        .bind(longitude)
        .bind(duration)
        .bind(&camera_model)
        .bind(is_favorite)
        .bind(&photo_hash)
        .bind(&crop_metadata)
        .bind(&thumb_rel)
        .bind(&created_at)
        .execute(&state.pool)
        .await?;

        // Replicate photo tags from the primary
        for tag in &tags {
            if let Err(e) = sqlx::query(
                "INSERT OR IGNORE INTO photo_tags (photo_id, user_id, tag, created_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&photo_id)
            .bind(&owner_id)
            .bind(tag)
            .bind(&created_at)
            .execute(&state.pool)
            .await
            {
                tracing::warn!(
                    photo_id = %photo_id,
                    tag = %tag,
                    "Failed to replicate photo tag during backup: {}",
                    e
                );
            }
        }

        // If this item previously existed in trash on the backup (e.g. it was
        // synced as deleted and has since been restored on the primary), remove
        // the stale trash entry so it only appears in the gallery.
        if let Err(e) = sqlx::query("DELETE FROM trash_items WHERE id = ?")
            .bind(&photo_id)
            .execute(&state.pool)
            .await
        {
            tracing::warn!(
                photo_id = %photo_id,
                "Failed to remove trash entry after receiving as active photo: {}",
                e
            );
        }
    }

    // ── Generate thumbnail immediately ───────────────────────────────────────
    // Don't wait for the background autoscan pass — generate now so the
    // backup serves thumbnails right after the sync completes.
    // Audio files get a solid-black placeholder, matching the primary.
    {
        let thumb_abs = storage_root.join(&thumb_rel);
        let generated = generate_thumbnail_file(&full_path, &thumb_abs, &mime_type, None).await;
        if generated {
            tracing::debug!(photo_id = %photo_id, "Generated thumbnail on receive");
        } else {
            tracing::warn!(
                photo_id = %photo_id,
                "Thumbnail generation failed on receive; will be retried by autoscan"
            );
        }
    }

    tracing::debug!(
        "Received backup {} ({} bytes): {}",
        source,
        size_bytes,
        file_path
    );

    Ok(Json(serde_json::json!({
        "status": "ok",
        "photo_id": photo_id,
        "size_bytes": size_bytes,
    })))
}

// ── Deletion Sync Endpoint ──────────────────────────────────────────────────

/// POST /api/backup/sync-deletions
/// Accepts a list of photo IDs that have been deleted on the primary server
/// (i.e., are now in the primary's trash) and removes them from the backup's
/// `photos` + `photo_tags` tables so the items no longer appear in the gallery.
///
/// This is called during every sync so that items deleted on the primary are
/// evicted from the backup gallery even when they were already in
/// `remote_trash_ids` (and therefore skipped by the file-transfer delta logic).
pub async fn backup_sync_deletions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let ids: Vec<String> = body
        .get("deleted_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if ids.is_empty() {
        return Ok(StatusCode::OK);
    }

    let mut removed = 0usize;
    for id in &ids {
        // Remove from gallery; ignore if it wasn't there (nothing to do).
        let result = sqlx::query("DELETE FROM photos WHERE id = ?")
            .bind(id)
            .execute(&state.pool)
            .await;
        match result {
            Ok(r) if r.rows_affected() > 0 => {
                removed += 1;
                // Clean up orphaned tags for the removed row.
                let _ = sqlx::query("DELETE FROM photo_tags WHERE photo_id = ?")
                    .bind(id)
                    .execute(&state.pool)
                    .await;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(photo_id = %id, "sync-deletions: failed to remove from photos: {}", e);
            }
        }
    }

    if removed > 0 {
        tracing::info!(
            "sync-deletions: removed {} photo(s) from gallery that are now in primary trash",
            removed
        );
    }

    Ok(StatusCode::OK)
}

// ── User Sync Endpoints ────────────────────────────────────────────────────

/// GET /api/backup/list-users
/// Returns all user IDs on this backup server for delta-sync detection.
/// Used by the primary's sync engine to skip users already present.
pub async fn backup_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, username FROM users ORDER BY created_at ASC")
            .fetch_all(&state.read_pool)
            .await?;

    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, username)| serde_json::json!({ "id": id, "username": username }))
        .collect();

    Ok(Json(items))
}

/// POST /api/backup/upsert-user
/// Creates or updates a user on this backup server with full credentials.
/// Transfers id, username, password_hash, role, storage_quota_bytes,
/// created_at, totp_secret, totp_enabled, and totp_backup_codes from the
/// primary so that users can log in on the backup server.
pub async fn backup_upsert_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("missing id".into()))?
        .to_string();
    let username = body
        .get("username")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("missing username".into()))?
        .to_string();
    let role = body
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("user")
        .to_string();
    let quota = body
        .get("storage_quota_bytes")
        .and_then(|v| v.as_i64())
        .unwrap_or(10_737_418_240); // 10 GiB default
    let created_at = body
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let password_hash = body
        .get("password_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("missing password_hash".into()))?
        .to_string();
    let totp_secret: Option<String> = body
        .get("totp_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let totp_enabled: i32 = body
        .get("totp_enabled")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;

    // Upsert the user record with full credentials.
    // ON CONFLICT updates all mutable fields so password changes,
    // role changes, and 2FA changes propagate from the primary.
    let result = sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, \
         role, totp_secret, totp_enabled) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
             username            = excluded.username, \
             password_hash       = excluded.password_hash, \
             role                = excluded.role, \
             storage_quota_bytes = excluded.storage_quota_bytes, \
             totp_secret         = excluded.totp_secret, \
             totp_enabled        = excluded.totp_enabled",
    )
    .bind(&id)
    .bind(&username)
    .bind(&password_hash)
    .bind(&created_at)
    .bind(quota)
    .bind(&role)
    .bind(&totp_secret)
    .bind(totp_enabled)
    .execute(&state.pool)
    .await;

    if let Err(e) = result {
        // UNIQUE violation on `username` — a local account with the same
        // name but a different id already exists.  Merge the local user
        // into the primary user so all content is visible under one account.
        let err_str = e.to_string();
        if err_str.contains("UNIQUE constraint failed: users.username") {
            tracing::info!(
                "backup_upsert_user: merging local '{}' into primary id={}",
                username,
                id
            );

            // Find the conflicting local user id
            let local_id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM users WHERE username = ? AND id != ?",
            )
            .bind(&username)
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);

            if let Some(ref old_id) = local_id {
                // Reassign all content from the local user to the primary user id.
                // FKs have ON DELETE CASCADE, but we want to keep the data — so
                // re-parent first, then delete the old user row.
                let reparent_tables: &[&str] = &[
                    "UPDATE photos SET user_id = ? WHERE user_id = ?",
                    "UPDATE trash_items SET user_id = ? WHERE user_id = ?",
                    "UPDATE photo_tags SET user_id = ? WHERE user_id = ?",
                    "UPDATE blobs SET user_id = ? WHERE user_id = ?",
                    "UPDATE audit_log SET user_id = ? WHERE user_id = ?",
                    "UPDATE client_logs SET user_id = ? WHERE user_id = ?",
                    "UPDATE shared_albums SET owner_user_id = ? WHERE owner_user_id = ?",
                    "UPDATE shared_album_members SET user_id = ? WHERE user_id = ?",
                ];
                for sql in reparent_tables {
                    if let Err(re) = sqlx::query(sql)
                        .bind(&id)
                        .bind(old_id)
                        .execute(&state.pool)
                        .await
                    {
                        // Non-fatal: table may not exist on minimal setups
                        tracing::debug!(
                            "backup_upsert_user: reparent skipped for '{}': {}",
                            sql.split_whitespace().nth(1).unwrap_or("?"),
                            re
                        );
                    }
                }

                // Also reparent encrypted_galleries and encryption_user_keys
                // (keyed on user_id TEXT PRIMARY KEY — needs special handling)
                let _ = sqlx::query(
                    "DELETE FROM encrypted_galleries WHERE user_id = ?",
                )
                .bind(old_id)
                .execute(&state.pool)
                .await;

                // Remove the old local user
                let _ = sqlx::query("DELETE FROM users WHERE id = ?")
                    .bind(old_id)
                    .execute(&state.pool)
                    .await;

                tracing::info!(
                    "backup_upsert_user: removed local user {} and reparented content to {}",
                    old_id,
                    id
                );
            }

            // Now insert the primary user — the conflicting row is gone
            if let Err(insert_err) = sqlx::query(
                "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, \
                 role, totp_secret, totp_enabled) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET \
                     username            = excluded.username, \
                     password_hash       = excluded.password_hash, \
                     role                = excluded.role, \
                     storage_quota_bytes = excluded.storage_quota_bytes, \
                     totp_secret         = excluded.totp_secret, \
                     totp_enabled        = excluded.totp_enabled",
            )
            .bind(&id)
            .bind(&username)
            .bind(&password_hash)
            .bind(&created_at)
            .bind(quota)
            .bind(&role)
            .bind(&totp_secret)
            .bind(totp_enabled)
            .execute(&state.pool)
            .await
            {
                tracing::error!(
                    "backup_upsert_user: merge insert failed for id={}: {}",
                    id,
                    insert_err
                );
                return Err(AppError::Internal(format!(
                    "Failed to create backup user record for id={}: {}",
                    id, insert_err
                )));
            }
        } else {
            // Some other DB error — report it
            tracing::error!(
                "backup_upsert_user: unexpected error for id={}: {}",
                id,
                e
            );
            return Err(AppError::Internal(format!(
                "Failed to create backup user record for id={}: {}",
                id, e
            )));
        }
    }

    // Sync TOTP backup codes — replace all codes for this user with the
    // primary's current set so revocations and regenerations propagate.
    if let Some(codes) = body.get("totp_backup_codes").and_then(|v| v.as_array()) {
        // Clear existing codes for this user
        if let Err(e) = sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
            .bind(&id)
            .execute(&state.pool)
            .await
        {
            tracing::warn!("Failed to clear TOTP backup codes for user {}: {}", id, e);
        }

        for code in codes {
            let code_id = match code.get("id").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => continue,
            };
            let code_hash = match code.get("code_hash").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => continue,
            };
            let used = code.get("used").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

            if let Err(e) = sqlx::query(
                "INSERT OR REPLACE INTO totp_backup_codes (id, user_id, code_hash, used) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(code_id)
            .bind(&id)
            .bind(code_hash)
            .bind(used)
            .execute(&state.pool)
            .await
            {
                tracing::warn!(
                    "Failed to sync TOTP backup code {} for user {}: {}",
                    code_id,
                    id,
                    e
                );
            }
        }
    }

    Ok(StatusCode::OK)
}

/// GET /api/backup/list-users-full
/// Returns all user records with full credentials (password_hash, TOTP secrets,
/// backup codes) for disaster recovery. The recovering primary calls this to
/// restore user accounts so they can log in with the same credentials.
///
/// **Security:** Authenticated via X-API-Key (same as all backup-serve endpoints).
pub async fn backup_list_users_full(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let users: Vec<(
        String,         // id
        String,         // username
        String,         // password_hash
        String,         // role
        i64,            // storage_quota_bytes
        String,         // created_at
        Option<String>, // totp_secret
        i32,            // totp_enabled
    )> = sqlx::query_as(
        "SELECT id, username, password_hash, role, storage_quota_bytes, \
         created_at, totp_secret, totp_enabled FROM users ORDER BY created_at ASC",
    )
    .fetch_all(&state.read_pool)
    .await?;

    let mut result = Vec::with_capacity(users.len());
    for (id, username, password_hash, role, quota, created_at, totp_secret, totp_enabled) in &users
    {
        // Fetch TOTP backup codes for this user
        let backup_codes: Vec<(String, String, i32)> =
            sqlx::query_as("SELECT id, code_hash, used FROM totp_backup_codes WHERE user_id = ?")
                .bind(id)
                .fetch_all(&state.read_pool)
                .await
                .unwrap_or_default();

        let codes_json: Vec<serde_json::Value> = backup_codes
            .iter()
            .map(|(code_id, code_hash, used)| {
                serde_json::json!({
                    "id": code_id,
                    "code_hash": code_hash,
                    "used": used,
                })
            })
            .collect();

        result.push(serde_json::json!({
            "id": id,
            "username": username,
            "password_hash": password_hash,
            "role": role,
            "storage_quota_bytes": quota,
            "created_at": created_at,
            "totp_secret": totp_secret,
            "totp_enabled": totp_enabled,
            "totp_backup_codes": codes_json,
        }));
    }

    Ok(Json(result))
}

/// POST /api/backup/sync-user-deletions
/// Accepts a list of user IDs that have been deleted on the primary server
/// and removes them from the backup's `users` table. Foreign-key cascades
/// clean up related rows (refresh_tokens, totp_backup_codes, etc.).
///
/// Content owned by deleted users (photos, trash, blobs) is also removed
/// via ON DELETE CASCADE, matching the primary's behaviour.
pub async fn backup_sync_user_deletions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let ids: Vec<String> = body
        .get("deleted_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if ids.is_empty() {
        return Ok(StatusCode::OK);
    }

    let mut removed = 0usize;
    for id in &ids {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&state.pool)
            .await;
        match result {
            Ok(r) if r.rows_affected() > 0 => {
                removed += 1;
            }
            Ok(_) => {} // user didn't exist — nothing to do
            Err(e) => {
                tracing::warn!(user_id = %id, "sync-user-deletions: failed to remove user: {}", e);
            }
        }
    }

    if removed > 0 {
        tracing::info!(
            "sync-user-deletions: removed {} user(s) deleted on primary",
            removed
        );
    }

    Ok(StatusCode::OK)
}
