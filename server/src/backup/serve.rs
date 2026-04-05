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

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use percent_encoding::{utf8_percent_encode, CONTROLS};

use crate::error::AppError;
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
pub(super) async fn validate_api_key(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
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
        // Remove from gallery by id OR encrypted_blob_id — for encrypted
        // items the trash stores blob_id as photo_id, which maps to
        // photos.encrypted_blob_id rather than photos.id on the backup.
        let result = sqlx::query(
            "DELETE FROM photos WHERE id = ? OR encrypted_blob_id = ?",
        )
        .bind(id)
        .bind(id)
        .execute(&state.pool)
        .await;
        match result {
            Ok(r) if r.rows_affected() > 0 => {
                removed += r.rows_affected() as usize;
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

// ── Secure Gallery Sync Endpoint ─────────────────────────────────────────────

/// POST /api/backup/sync-secure-galleries
/// Receives the full state of `encrypted_galleries` and `encrypted_gallery_items`
/// from the primary.  Upserts all rows and removes any that no longer exist on
/// the primary (full-state replacement).
///
/// This ensures the backup knows which `photos` rows are secure-album clones
/// and can filter them from the regular gallery view via `encrypted-sync`.
pub async fn backup_sync_secure_galleries(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let galleries = body
        .get("galleries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut tx = state.pool.begin().await?;

    // Upsert galleries
    let mut gallery_ids = std::collections::HashSet::new();
    for g in &galleries {
        let id = g["id"].as_str().unwrap_or_default();
        let user_id = g["user_id"].as_str().unwrap_or_default();
        let name = g["name"].as_str().unwrap_or_default();
        let password_hash = g["password_hash"].as_str().unwrap_or("account-auth");
        let created_at = g["created_at"].as_str().unwrap_or_default();

        if id.is_empty() || user_id.is_empty() {
            continue;
        }

        gallery_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO encrypted_galleries (id, user_id, name, password_hash, created_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name, \
               password_hash = excluded.password_hash",
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(password_hash)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }

    // Upsert items
    let mut item_ids = std::collections::HashSet::new();
    for i in &items {
        let id = i["id"].as_str().unwrap_or_default();
        let gallery_id = i["gallery_id"].as_str().unwrap_or_default();
        let blob_id = i["blob_id"].as_str().unwrap_or_default();
        let added_at = i["added_at"].as_str().unwrap_or_default();
        let original_blob_id = i["original_blob_id"].as_str();

        if id.is_empty() || gallery_id.is_empty() || blob_id.is_empty() {
            continue;
        }

        item_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at, original_blob_id) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               blob_id = excluded.blob_id, \
               original_blob_id = excluded.original_blob_id",
        )
        .bind(id)
        .bind(gallery_id)
        .bind(blob_id)
        .bind(added_at)
        .bind(original_blob_id)
        .execute(&mut *tx)
        .await?;
    }

    // Remove galleries/items that no longer exist on the primary.
    // Only prune if the primary sent at least one gallery (avoid wiping
    // everything when the request is empty due to a transient error).
    if !gallery_ids.is_empty() {
        let existing_gallery_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM encrypted_galleries")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();

        for existing_id in &existing_gallery_ids {
            if !gallery_ids.contains(existing_id) {
                sqlx::query("DELETE FROM encrypted_galleries WHERE id = ?")
                    .bind(existing_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    if !item_ids.is_empty() {
        let existing_item_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM encrypted_gallery_items")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();

        for existing_id in &existing_item_ids {
            if !item_ids.contains(existing_id) {
                sqlx::query("DELETE FROM encrypted_gallery_items WHERE id = ?")
                    .bind(existing_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    tx.commit().await?;

    tracing::info!(
        "Received secure gallery sync: {} galleries, {} items",
        galleries.len(),
        items.len()
    );

    Ok(StatusCode::OK)
}

// ── Blob List Endpoint ───────────────────────────────────────────────────────

/// GET /api/backup/list-blobs
/// Returns a list of all blob IDs on this server for delta sync.
pub async fn backup_list_blobs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let blobs: Vec<(String, i64)> = sqlx::query_as(
        "SELECT id, size_bytes FROM blobs ORDER BY upload_time ASC",
    )
    .fetch_all(&state.read_pool)
    .await?;

    let result: Vec<serde_json::Value> = blobs
        .iter()
        .map(|(id, size)| serde_json::json!({ "id": id, "size_bytes": size }))
        .collect();

    Ok(Json(result))
}

// ── Blob Receive Endpoint ────────────────────────────────────────────────────

/// POST /api/backup/receive-blob
/// Receives a client-encrypted blob file pushed from the primary server.
/// Writes the file to disk and upserts the `blobs` table metadata.
pub async fn backup_receive_blob(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let blob_id = headers
        .get("X-Blob-Id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-Blob-Id header".into()))?
        .to_string();

    let storage_path_raw = headers
        .get("X-Storage-Path")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing X-Storage-Path header".into()))?;

    let storage_path = percent_encoding::percent_decode_str(storage_path_raw)
        .decode_utf8()
        .map_err(|_| AppError::BadRequest("Invalid X-Storage-Path encoding".into()))?
        .to_string();

    // Security: validate path to prevent traversal
    crate::sanitize::validate_relative_path(&storage_path)
        .map_err(|e| AppError::BadRequest(format!("Invalid storage path: {}", e)))?;

    // Verify checksum
    if let Some(expected) = headers.get("X-Content-Hash").and_then(|v| v.to_str().ok()) {
        use sha2::Digest;
        let actual = hex::encode(sha2::Sha256::digest(&body));
        if actual != expected {
            return Err(AppError::BadRequest("Content hash mismatch".into()));
        }
    }

    // Parse metadata headers
    let user_id = headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let blob_type = headers
        .get("X-Blob-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("photo")
        .to_string();
    let upload_time = headers
        .get("X-Upload-Time")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let size_bytes = headers
        .get("X-Size-Bytes")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(body.len() as i64);
    let client_hash: Option<String> = headers
        .get("X-Client-Hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let content_hash: Option<String> = headers
        .get("X-Original-Content-Hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Write file to disk
    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&storage_path);
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create blob dir: {}", e)))?;
    }
    tokio::fs::write(&full_path, &body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write blob: {}", e)))?;

    // Upsert blob metadata — fall back to admin if user doesn't exist locally
    let effective_user_id = if user_id.is_empty() {
        sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE role = 'admin' LIMIT 1")
            .fetch_optional(&state.pool)
            .await?
            .unwrap_or_default()
    } else {
        let exists: bool =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE id = ?)")
                .bind(&user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(false);
        if exists {
            user_id
        } else {
            sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE role = 'admin' LIMIT 1")
                .fetch_optional(&state.pool)
                .await?
                .unwrap_or_default()
        }
    };

    if effective_user_id.is_empty() {
        tracing::warn!("receive-blob: no valid user for blob {}, skipping DB insert", blob_id);
        return Ok(StatusCode::OK);
    }

    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
           blob_type = excluded.blob_type, \
           size_bytes = excluded.size_bytes, \
           client_hash = excluded.client_hash, \
           content_hash = excluded.content_hash",
    )
    .bind(&blob_id)
    .bind(&effective_user_id)
    .bind(&blob_type)
    .bind(size_bytes)
    .bind(&client_hash)
    .bind(&upload_time)
    .bind(&storage_path)
    .bind(&content_hash)
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

// ── Metadata Sync Endpoint ───────────────────────────────────────────────────

/// POST /api/backup/sync-metadata
/// Receives the full state of metadata tables from the primary.
/// Full-state sync: upserts all rows, deletes rows no longer on primary.
///
/// Tables synced: edit_copies, photo_metadata, shared_albums,
///                shared_album_members, shared_album_photos
pub async fn backup_sync_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let mut tx = state.pool.begin().await?;

    // ── edit_copies ──────────────────────────────────────────────────────
    let edit_copies = body
        .get("edit_copies")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut ec_ids = std::collections::HashSet::new();
    for row in &edit_copies {
        let id = row["id"].as_str().unwrap_or_default();
        let photo_id = row["photo_id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let name = row["name"].as_str().unwrap_or_default();
        let edit_metadata = row["edit_metadata"].as_str().unwrap_or("{}");
        let created_at = row["created_at"].as_str().unwrap_or_default();

        if id.is_empty() || photo_id.is_empty() {
            continue;
        }
        ec_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO edit_copies (id, photo_id, user_id, name, edit_metadata, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name, \
               edit_metadata = excluded.edit_metadata",
        )
        .bind(id)
        .bind(photo_id)
        .bind(user_id)
        .bind(name)
        .bind(edit_metadata)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }

    if !ec_ids.is_empty() {
        let existing: Vec<String> =
            sqlx::query_scalar("SELECT id FROM edit_copies")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();
        for eid in &existing {
            if !ec_ids.contains(eid) {
                sqlx::query("DELETE FROM edit_copies WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    // ── photo_metadata ───────────────────────────────────────────────────
    let photo_metadata = body
        .get("photo_metadata")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut pm_ids = std::collections::HashSet::new();
    for row in &photo_metadata {
        let id = row["id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let photo_id = row["photo_id"].as_str();
        let blob_id = row["blob_id"].as_str();
        let source = row["source"].as_str().unwrap_or("manual");
        let title = row["title"].as_str();
        let description = row["description"].as_str();
        let taken_at = row["taken_at"].as_str();
        let created_at_src = row["created_at_src"].as_str();
        let latitude = row["latitude"].as_f64();
        let longitude = row["longitude"].as_f64();
        let altitude = row["altitude"].as_f64();
        let image_views = row["image_views"].as_i64();
        let original_url = row["original_url"].as_str();
        let storage_path = row["storage_path"].as_str();
        let imported_at = row["imported_at"].as_str().unwrap_or_default();

        if id.is_empty() {
            continue;
        }
        pm_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO photo_metadata (id, user_id, photo_id, blob_id, source, title, \
             description, taken_at, created_at_src, latitude, longitude, altitude, \
             image_views, original_url, storage_path, imported_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               photo_id = excluded.photo_id, \
               blob_id = excluded.blob_id, \
               title = excluded.title, \
               description = excluded.description, \
               taken_at = excluded.taken_at, \
               latitude = excluded.latitude, \
               longitude = excluded.longitude",
        )
        .bind(id)
        .bind(user_id)
        .bind(photo_id)
        .bind(blob_id)
        .bind(source)
        .bind(title)
        .bind(description)
        .bind(taken_at)
        .bind(created_at_src)
        .bind(latitude)
        .bind(longitude)
        .bind(altitude)
        .bind(image_views)
        .bind(original_url)
        .bind(storage_path)
        .bind(imported_at)
        .execute(&mut *tx)
        .await?;
    }

    if !pm_ids.is_empty() {
        let existing: Vec<String> =
            sqlx::query_scalar("SELECT id FROM photo_metadata")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();
        for eid in &existing {
            if !pm_ids.contains(eid) {
                sqlx::query("DELETE FROM photo_metadata WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    // ── shared_albums ────────────────────────────────────────────────────
    let shared_albums = body
        .get("shared_albums")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut sa_ids = std::collections::HashSet::new();
    for row in &shared_albums {
        let id = row["id"].as_str().unwrap_or_default();
        let owner_user_id = row["owner_user_id"].as_str().unwrap_or_default();
        let name = row["name"].as_str().unwrap_or_default();
        let created_at = row["created_at"].as_str().unwrap_or_default();

        if id.is_empty() {
            continue;
        }
        sa_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO shared_albums (id, owner_user_id, name, created_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name",
        )
        .bind(id)
        .bind(owner_user_id)
        .bind(name)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }

    // ── shared_album_members ─────────────────────────────────────────────
    let shared_members = body
        .get("shared_album_members")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut sm_ids = std::collections::HashSet::new();
    for row in &shared_members {
        let id = row["id"].as_str().unwrap_or_default();
        let album_id = row["album_id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let added_at = row["added_at"].as_str().unwrap_or_default();

        if id.is_empty() {
            continue;
        }
        sm_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO shared_album_members (id, album_id, user_id, added_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               album_id = excluded.album_id, \
               user_id = excluded.user_id",
        )
        .bind(id)
        .bind(album_id)
        .bind(user_id)
        .bind(added_at)
        .execute(&mut *tx)
        .await?;
    }

    // ── shared_album_photos ──────────────────────────────────────────────
    let shared_photos = body
        .get("shared_album_photos")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut sp_ids = std::collections::HashSet::new();
    for row in &shared_photos {
        let id = row["id"].as_str().unwrap_or_default();
        let album_id = row["album_id"].as_str().unwrap_or_default();
        let photo_ref = row["photo_ref"].as_str().unwrap_or_default();
        let ref_type = row["ref_type"].as_str().unwrap_or("photo");
        let added_at = row["added_at"].as_str().unwrap_or_default();

        if id.is_empty() {
            continue;
        }
        sp_ids.insert(id.to_string());

        sqlx::query(
            "INSERT INTO shared_album_photos (id, album_id, photo_ref, ref_type, added_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               photo_ref = excluded.photo_ref, \
               ref_type = excluded.ref_type",
        )
        .bind(id)
        .bind(album_id)
        .bind(photo_ref)
        .bind(ref_type)
        .bind(added_at)
        .execute(&mut *tx)
        .await?;
    }

    // ── Prune deleted shared data ────────────────────────────────────────
    if !sp_ids.is_empty() {
        let existing: Vec<String> =
            sqlx::query_scalar("SELECT id FROM shared_album_photos")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();
        for eid in &existing {
            if !sp_ids.contains(eid) {
                sqlx::query("DELETE FROM shared_album_photos WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    if !sm_ids.is_empty() {
        let existing: Vec<String> =
            sqlx::query_scalar("SELECT id FROM shared_album_members")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();
        for eid in &existing {
            if !sm_ids.contains(eid) {
                sqlx::query("DELETE FROM shared_album_members WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    if !sa_ids.is_empty() {
        let existing: Vec<String> =
            sqlx::query_scalar("SELECT id FROM shared_albums")
                .fetch_all(&mut *tx)
                .await
                .unwrap_or_default();
        for eid in &existing {
            if !sa_ids.contains(eid) {
                sqlx::query("DELETE FROM shared_albums WHERE id = ?")
                    .bind(eid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    tx.commit().await?;

    tracing::info!(
        "Received metadata sync: {} edit_copies, {} photo_metadata, \
         {} shared_albums, {} members, {} album_photos",
        edit_copies.len(),
        photo_metadata.len(),
        shared_albums.len(),
        shared_members.len(),
        shared_photos.len(),
    );

    Ok(StatusCode::OK)
}
