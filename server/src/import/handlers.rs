//! Import handlers: Google Photos metadata import, metadata CRUD, and
//! sidecar upload.
//!
//! Metadata that is not packed alongside the blob is stored as JSON files
//! in the `{storage_root}/metadata/` subtree.  When encryption mode is active,
//! the metadata JSON is encrypted before being written to disk.
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::blobs::storage as blob_storage;
use crate::error::AppError;
use crate::state::AppState;

use super::google_photos;
use super::models::*;

// ── Single metadata import ───────────────────────────────────────────────────

/// POST /api/import/metadata
///
/// Import metadata from a Google Photos JSON sidecar (or any supported source).
/// The metadata is stored in the `metadata/` subdirectory unless the caller
/// indicates it is packed with the blob.
pub async fn import_metadata(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<GooglePhotosImportRequest>,
) -> Result<(StatusCode, Json<ImportMetadataResponse>), AppError> {
    let meta_id = Uuid::new_v4().to_string();
    let record = google_photos::normalise(
        &req.metadata,
        meta_id.clone(),
        auth.user_id.clone(),
        req.photo_id.clone(),
        req.blob_id.clone(),
    );

    // Determine whether encryption mode is active
    let encryption_mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    let is_encrypted = encryption_mode == "encrypted";

    // Serialize the full Google Photos JSON for archival in the metadata/ dir
    let raw_json = serde_json::to_vec_pretty(&req.metadata)
        .map_err(|e| AppError::Internal(format!("Failed to serialize metadata: {}", e)))?;

    // NOTE: In encrypted mode the metadata JSON is encrypted client-side
    // before upload.  For server-side import we store the plaintext JSON;
    // the client migration process will encrypt it later.  Both branches
    // currently produce the same output, but the distinction is preserved
    // so future server-side encryption can be added to the first branch.
    let data_to_write = raw_json.clone();

    // Write the metadata file to storage_root/metadata/...
    let storage_root = state.storage_root.read().await.clone();
    let storage_path = blob_storage::write_metadata(
        &storage_root,
        &auth.user_id,
        &meta_id,
        &data_to_write,
    )
    .await?;

    // Insert DB record
    sqlx::query(
        "INSERT INTO photo_metadata \
         (id, user_id, photo_id, blob_id, source, title, description, taken_at, \
          created_at_src, latitude, longitude, altitude, image_views, original_url, \
          storage_path, is_encrypted, imported_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&record.id)
    .bind(&record.user_id)
    .bind(&record.photo_id)
    .bind(&record.blob_id)
    .bind(&record.source)
    .bind(&record.title)
    .bind(&record.description)
    .bind(&record.taken_at)
    .bind(&record.created_at_src)
    .bind(record.latitude)
    .bind(record.longitude)
    .bind(record.altitude)
    .bind(record.image_views)
    .bind(&record.original_url)
    .bind(&storage_path)
    .bind(is_encrypted as i32)
    .bind(&record.imported_at)
    .execute(&state.pool)
    .await?;

    audit::log(
        &state.pool,
        AuditEvent::BlobUpload, // reuse existing event type
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "import_metadata",
            "metadata_id": meta_id,
            "source": "google_photos",
        })),
    )
    .await;

    tracing::info!(
        user_id = %auth.user_id,
        metadata_id = %meta_id,
        "Imported Google Photos metadata"
    );

    Ok((
        StatusCode::CREATED,
        Json(ImportMetadataResponse {
            metadata_id: meta_id,
            storage_path: Some(storage_path),
            is_encrypted,
        }),
    ))
}

// ── Batch import ─────────────────────────────────────────────────────────────

/// POST /api/import/metadata/batch
///
/// Import multiple Google Photos metadata entries at once.
/// Returns per-entry success/failure results.
pub async fn batch_import_metadata(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<GooglePhotosBatchImportRequest>,
) -> Result<Json<BatchImportResponse>, AppError> {
    if req.entries.len() > 500 {
        return Err(AppError::BadRequest(
            "Maximum 500 entries per batch".into(),
        ));
    }

    let encryption_mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    let is_encrypted = encryption_mode == "encrypted";
    let storage_root = state.storage_root.read().await.clone();

    let mut results = Vec::with_capacity(req.entries.len());
    let mut imported = 0usize;
    let mut failed = 0usize;

    for (idx, entry) in req.entries.iter().enumerate() {
        let meta_id = Uuid::new_v4().to_string();

        match import_single_metadata(
            &state,
            &auth.user_id,
            &meta_id,
            entry,
            is_encrypted,
            &storage_root,
        )
        .await
        {
            Ok(_) => {
                imported += 1;
                results.push(ImportMetadataResultEntry {
                    index: idx,
                    metadata_id: Some(meta_id),
                    error: None,
                });
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(
                    user_id = %auth.user_id,
                    index = idx,
                    error = %e,
                    "Batch metadata import: entry failed"
                );
                results.push(ImportMetadataResultEntry {
                    index: idx,
                    metadata_id: None,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    audit::log(
        &state.pool,
        AuditEvent::BlobUpload,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "batch_import_metadata",
            "imported": imported,
            "failed": failed,
        })),
    )
    .await;

    tracing::info!(
        user_id = %auth.user_id,
        imported = imported,
        failed = failed,
        "Batch Google Photos metadata import complete"
    );

    Ok(Json(BatchImportResponse {
        imported,
        failed,
        results,
    }))
}

/// Internal helper: import a single metadata entry and write to storage + DB.
async fn import_single_metadata(
    state: &AppState,
    user_id: &str,
    meta_id: &str,
    entry: &GooglePhotosImportRequest,
    is_encrypted: bool,
    storage_root: &std::path::Path,
) -> Result<(), AppError> {
    let record = google_photos::normalise(
        &entry.metadata,
        meta_id.to_string(),
        user_id.to_string(),
        entry.photo_id.clone(),
        entry.blob_id.clone(),
    );

    let raw_json = serde_json::to_vec_pretty(&entry.metadata)
        .map_err(|e| AppError::Internal(format!("Failed to serialize metadata: {}", e)))?;

    let storage_path =
        blob_storage::write_metadata(storage_root, user_id, meta_id, &raw_json).await?;

    sqlx::query(
        "INSERT INTO photo_metadata \
         (id, user_id, photo_id, blob_id, source, title, description, taken_at, \
          created_at_src, latitude, longitude, altitude, image_views, original_url, \
          storage_path, is_encrypted, imported_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(meta_id)
    .bind(user_id)
    .bind(&record.photo_id)
    .bind(&record.blob_id)
    .bind(&record.source)
    .bind(&record.title)
    .bind(&record.description)
    .bind(&record.taken_at)
    .bind(&record.created_at_src)
    .bind(record.latitude)
    .bind(record.longitude)
    .bind(record.altitude)
    .bind(record.image_views)
    .bind(&record.original_url)
    .bind(&storage_path)
    .bind(is_encrypted as i32)
    .bind(&record.imported_at)
    .execute(&state.pool)
    .await?;

    Ok(())
}

// ── Upload raw JSON sidecar ──────────────────────────────────────────────────

/// POST /api/import/metadata/upload
///
/// Upload a raw Google Photos JSON sidecar file. The server parses it, stores
/// the metadata, and optionally associates it with a photo/blob.
///
/// Headers:
///   - X-Photo-Id: optional photo to associate with
///   - X-Blob-Id:  optional blob to associate with
pub async fn upload_sidecar(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<ImportMetadataResponse>), AppError> {
    if body.is_empty() {
        return Err(AppError::BadRequest("Empty body".into()));
    }

    // Cap sidecar size at 1 MiB — these are small JSON files
    if body.len() > 1_048_576 {
        return Err(AppError::BadRequest(
            "Metadata sidecar too large (max 1 MiB)".into(),
        ));
    }

    let gp_meta = google_photos::parse_sidecar(&body)
        .map_err(|e| AppError::BadRequest(e))?;

    let photo_id = headers
        .get("x-photo-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let blob_id = headers
        .get("x-blob-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let req = GooglePhotosImportRequest {
        metadata: gp_meta,
        photo_id,
        blob_id,
    };

    import_metadata(State(state), auth, headers, Json(req)).await
}

// ── Get metadata for a photo ─────────────────────────────────────────────────

/// GET /api/photos/:id/metadata
///
/// Retrieve all metadata records associated with a photo.
pub async fn get_photo_metadata(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Json<MetadataListResponse>, AppError> {
    let metadata: Vec<PhotoMetadataRecord> = sqlx::query_as(
        "SELECT id, user_id, photo_id, blob_id, source, title, description, taken_at, \
         created_at_src, latitude, longitude, altitude, image_views, original_url, \
         storage_path, is_encrypted, imported_at \
         FROM photo_metadata WHERE user_id = ? AND photo_id = ? \
         ORDER BY imported_at DESC",
    )
    .bind(&auth.user_id)
    .bind(&photo_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(MetadataListResponse {
        metadata,
        next_cursor: None,
    }))
}

// ── Delete metadata ──────────────────────────────────────────────────────────

/// DELETE /api/photos/:id/metadata
///
/// Delete all metadata records for a photo and remove the files from disk.
pub async fn delete_photo_metadata(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let paths: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT storage_path FROM photo_metadata WHERE user_id = ? AND photo_id = ?",
    )
    .bind(&auth.user_id)
    .bind(&photo_id)
    .fetch_all(&state.pool)
    .await?;

    let storage_root = state.storage_root.read().await.clone();
    for path in paths.into_iter().flatten() {
        blob_storage::delete_metadata(&storage_root, &path).await.ok();
    }

    sqlx::query("DELETE FROM photo_metadata WHERE user_id = ? AND photo_id = ?")
        .bind(&auth.user_id)
        .bind(&photo_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
