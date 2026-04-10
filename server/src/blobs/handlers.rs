//! Encrypted blob storage endpoints.
//!
//! Handles upload (with SHA-256 integrity check and per-user quota
//! enforcement), paginated listing, and deletion with audit logging.
//! Streaming download lives in [`super::download`].

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::*;
use super::storage;

/// All valid blob types.  The server treats blobs as opaque encrypted bytes —
/// the type is stored as metadata only, for client-side querying.
const VALID_BLOB_TYPES: &[&str] = &[
    "photo",
    "gif",
    "video",
    "audio",
    "thumbnail",
    "video_thumbnail",
    "album_manifest",
];

/// Query parameters for the blob list endpoint.
#[derive(Debug, Deserialize)]
pub struct ListBlobsQuery {
    /// Filter by blob type (e.g. "photo", "video", "thumbnail").
    pub blob_type: Option<String>,
    /// Cursor for pagination — `upload_time` of the last item from the previous page.
    pub after: Option<String>,
    /// Maximum items to return (default 50, max 200).
    pub limit: Option<i64>,
}

/// POST /api/blobs — upload an encrypted blob.
///
/// Headers:
/// - `x-blob-type` — one of: photo, gif, video, audio, thumbnail,
///   video_thumbnail, album_manifest (default: "photo")
/// - `x-client-hash` — optional SHA-256 hex digest for integrity verification
/// - `x-content-hash` — optional short hash of the *original* (pre-encryption)
///   content, used for cross-platform photo alignment
///
/// Enforces per-user storage quota. Returns 201 with the new blob ID.
///
/// **Streaming:** The request body is streamed directly to disk in chunks
/// while simultaneously computing the SHA-256 hash.  This avoids buffering
/// multi-gigabyte video blobs entirely in server RAM.
pub async fn upload(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<BlobUploadResponse>), AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let blob_type = headers
        .get("x-blob-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("photo")
        .to_string();

    tracing::info!(
        user_id = %auth.user_id,
        blob_type = %blob_type,
        "Blob upload started (streaming)"
    );

    // Validate blob type against allowlist
    if !VALID_BLOB_TYPES.contains(&blob_type.as_str()) {
        tracing::warn!(
            user_id = %auth.user_id,
            blob_type = %blob_type,
            "Blob upload rejected: invalid blob type"
        );
        return Err(AppError::BadRequest(format!(
            "Invalid blob type '{}'. Valid types: {}",
            blob_type,
            VALID_BLOB_TYPES.join(", ")
        )));
    }

    let client_hash = headers
        .get("x-client-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // X-Content-Hash: short hash of the ORIGINAL (pre-encryption) content.
    // Used for cross-platform photo alignment — same original photo always
    // produces the same content_hash regardless of encryption nonce.
    let content_hash = headers
        .get("x-content-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // ── Pre-flight quota check (fast reject using Content-Length header) ─────
    // This avoids streaming the entire body only to reject it at the end.
    // The final size is re-verified after streaming completes.
    let used: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM blobs WHERE user_id = ?")
            .bind(&auth.user_id)
            .fetch_one(&state.read_pool)
            .await?;

    let quota: i64 = sqlx::query_scalar("SELECT storage_quota_bytes FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.read_pool)
        .await?;

    if let Some(cl) = headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
    {
        if cl > state.config.storage.max_blob_size_bytes as i64 {
            return Err(AppError::PayloadTooLarge);
        }
        if quota > 0 && used + cl > quota {
            return Err(AppError::Forbidden("Storage quota exceeded".into()));
        }
    }

    // ── Stream body to disk, computing SHA-256 incrementally ────────────────
    let blob_id = Uuid::new_v4().to_string();
    let storage_root = (**state.storage_root.load()).clone();
    let (storage_path, actual_size, computed_hash) =
        storage::write_blob_streaming(&storage_root, &auth.user_id, &blob_id, body).await?;

    // ── Post-stream validation ──────────────────────────────────────────────
    let cleanup = || async {
        if let Err(e) = storage::delete_blob(&storage_root, &storage_path).await {
            tracing::warn!("Failed to clean up blob at {}: {}", storage_path, e);
        }
    };

    if actual_size == 0 {
        cleanup().await;
        return Err(AppError::BadRequest("Empty blob body".into()));
    }

    if actual_size as i64 > state.config.storage.max_blob_size_bytes as i64 {
        cleanup().await;
        return Err(AppError::PayloadTooLarge);
    }

    // Final quota check with actual streamed size
    if quota > 0 && used + actual_size as i64 > quota {
        cleanup().await;
        return Err(AppError::Forbidden("Storage quota exceeded".into()));
    }

    // Server-side integrity check — compare streamed SHA-256 against client hash
    if let Some(ref expected_hash) = client_hash {
        if computed_hash != *expected_hash {
            tracing::warn!(
                user_id = auth.user_id,
                expected = expected_hash,
                computed = computed_hash,
                "Blob integrity check failed — hash mismatch"
            );
            cleanup().await;
            return Err(AppError::BadRequest(
                "Blob integrity check failed: X-Client-Hash does not match uploaded data".into(),
            ));
        }
    }

    // ── Content-hash dedup ──────────────────────────────────────────────────
    // If the caller provided X-Content-Hash (short hash of the *original*
    // unencrypted data), check whether this user already has a blob with the
    // same content_hash.  Return the existing blob instead of storing a
    // duplicate, mirroring the photo upload dedup behaviour.
    if let Some(ref ch) = content_hash {
        let existing: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT id, upload_time, size_bytes FROM blobs \
             WHERE user_id = ? AND content_hash = ? LIMIT 1",
        )
        .bind(&auth.user_id)
        .bind(ch)
        .fetch_optional(&state.read_pool)
        .await?;

        if let Some((eid, etime, esize)) = existing {
            tracing::info!(
                user_id = %auth.user_id,
                existing_blob_id = %eid,
                content_hash = %ch,
                "Duplicate blob upload detected (content_hash match) — returning existing record"
            );
            // Clean up the file we just wrote — it's a duplicate
            cleanup().await;
            return Ok((
                StatusCode::OK,
                Json(BlobUploadResponse {
                    blob_id: eid,
                    upload_time: etime,
                    size: esize,
                }),
            ));
        }
    }

    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .bind(&blob_type)
    .bind(actual_size as i64)
    .bind(&client_hash)
    .bind(&now)
    .bind(&storage_path)
    .bind(&content_hash)
    .execute(&state.pool)
    .await?;

    audit::log(
        &state,
        AuditEvent::BlobUpload,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "blob_id": blob_id,
            "blob_type": blob_type,
            "size_bytes": actual_size
        })),
    )
    .await;

    tracing::info!(
        user_id = %auth.user_id,
        blob_id = %blob_id,
        blob_type = %blob_type,
        size_bytes = actual_size,
        "Blob upload completed successfully"
    );

    Ok((
        StatusCode::CREATED,
        Json(BlobUploadResponse {
            blob_id,
            upload_time: now,
            size: actual_size as i64,
        }),
    ))
}

/// GET /api/blobs — list blobs for the authenticated user with cursor-based pagination.
/// Supports filtering by `blob_type` and forward-only cursor via `after`.
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ListBlobsQuery>,
) -> Result<Json<BlobListResponse>, AppError> {
    let limit = params.limit.unwrap_or(50).min(200);

    let blobs = if let Some(ref blob_type) = params.blob_type {
        // Validate blob_type query parameter
        if !VALID_BLOB_TYPES.contains(&blob_type.as_str()) {
            return Err(AppError::BadRequest(format!(
                "Invalid blob_type filter '{}'. Valid: {}",
                blob_type,
                VALID_BLOB_TYPES.join(", ")
            )));
        }

        if let Some(ref after) = params.after {
            sqlx::query_as::<_, BlobRecord>(
                "SELECT id, blob_type, size_bytes, client_hash, upload_time, content_hash FROM blobs \
                 WHERE user_id = ? AND blob_type = ? AND upload_time > ? \
                 AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
                 AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
                 AND id NOT IN ( \
                     SELECT p.encrypted_blob_id FROM photos p \
                     WHERE p.encrypted_blob_id IS NOT NULL \
                     AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                          OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
                 AND id NOT IN ( \
                     SELECT p.encrypted_thumb_blob_id FROM photos p \
                     WHERE p.encrypted_thumb_blob_id IS NOT NULL \
                     AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                          OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
                 AND id NOT IN (SELECT encrypted_blob_id FROM encrypted_gallery_items WHERE encrypted_blob_id IS NOT NULL) \
                 AND id NOT IN (SELECT encrypted_thumb_blob_id FROM encrypted_gallery_items WHERE encrypted_thumb_blob_id IS NOT NULL) \
                 ORDER BY upload_time ASC LIMIT ?",
            )
            .bind(&auth.user_id)
            .bind(blob_type)
            .bind(after)
            .bind(limit + 1)
            .fetch_all(&state.read_pool)
            .await?
        } else {
            sqlx::query_as::<_, BlobRecord>(
                "SELECT id, blob_type, size_bytes, client_hash, upload_time, content_hash FROM blobs \
                 WHERE user_id = ? AND blob_type = ? \
                 AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
                 AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
                 AND id NOT IN ( \
                     SELECT p.encrypted_blob_id FROM photos p \
                     WHERE p.encrypted_blob_id IS NOT NULL \
                     AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                          OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
                 AND id NOT IN ( \
                     SELECT p.encrypted_thumb_blob_id FROM photos p \
                     WHERE p.encrypted_thumb_blob_id IS NOT NULL \
                     AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                          OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
                 AND id NOT IN (SELECT encrypted_blob_id FROM encrypted_gallery_items WHERE encrypted_blob_id IS NOT NULL) \
                 AND id NOT IN (SELECT encrypted_thumb_blob_id FROM encrypted_gallery_items WHERE encrypted_thumb_blob_id IS NOT NULL) \
                 ORDER BY upload_time ASC LIMIT ?",
            )
            .bind(&auth.user_id)
            .bind(blob_type)
            .bind(limit + 1)
            .fetch_all(&state.read_pool)
            .await?
        }
    } else if let Some(ref after) = params.after {
        sqlx::query_as::<_, BlobRecord>(
            "SELECT id, blob_type, size_bytes, client_hash, upload_time, content_hash FROM blobs \
             WHERE user_id = ? AND upload_time > ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
             AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
             AND id NOT IN ( \
                 SELECT p.encrypted_blob_id FROM photos p \
                 WHERE p.encrypted_blob_id IS NOT NULL \
                 AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                      OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
             AND id NOT IN ( \
                 SELECT p.encrypted_thumb_blob_id FROM photos p \
                 WHERE p.encrypted_thumb_blob_id IS NOT NULL \
                 AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                      OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
             AND id NOT IN (SELECT encrypted_blob_id FROM encrypted_gallery_items WHERE encrypted_blob_id IS NOT NULL) \
             AND id NOT IN (SELECT encrypted_thumb_blob_id FROM encrypted_gallery_items WHERE encrypted_thumb_blob_id IS NOT NULL) \
             ORDER BY upload_time ASC LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(after)
        .bind(limit + 1)
        .fetch_all(&state.read_pool)
        .await?
    } else {
        sqlx::query_as::<_, BlobRecord>(
            "SELECT id, blob_type, size_bytes, client_hash, upload_time, content_hash FROM blobs \
             WHERE user_id = ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
             AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
             AND id NOT IN ( \
                 SELECT p.encrypted_blob_id FROM photos p \
                 WHERE p.encrypted_blob_id IS NOT NULL \
                 AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                      OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
             AND id NOT IN ( \
                 SELECT p.encrypted_thumb_blob_id FROM photos p \
                 WHERE p.encrypted_thumb_blob_id IS NOT NULL \
                 AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                      OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
             AND id NOT IN (SELECT encrypted_blob_id FROM encrypted_gallery_items WHERE encrypted_blob_id IS NOT NULL) \
             AND id NOT IN (SELECT encrypted_thumb_blob_id FROM encrypted_gallery_items WHERE encrypted_thumb_blob_id IS NOT NULL) \
             ORDER BY upload_time ASC LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(limit + 1)
        .fetch_all(&state.read_pool)
        .await?
    };

    let next_cursor = if blobs.len() as i64 > limit {
        blobs.last().map(|b| b.upload_time.clone())
    } else {
        None
    };

    let blobs: Vec<BlobRecord> = blobs.into_iter().take(limit as usize).collect();

    Ok(Json(BlobListResponse { blobs, next_cursor }))
}

/// DELETE /api/blobs/:id — delete a blob and its on-disk file. Returns 204 on success.
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(blob_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    // Validate blob_id format
    if Uuid::parse_str(&blob_id).is_err() {
        return Err(AppError::BadRequest("Invalid blob ID format".into()));
    }

    let storage_path = sqlx::query_scalar::<_, String>(
        "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    storage::delete_blob(&storage_root, &storage_path).await?;

    // Wrap DB operations in a transaction for atomicity
    let mut tx = state.pool.begin().await?;

    sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
        .bind(&blob_id)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    // Clean up shared album references to prevent dangling photo_ref entries
    sqlx::query("DELETE FROM shared_album_photos WHERE photo_ref = ? AND ref_type = 'blob'")
        .bind(&blob_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    audit::log(
        &state,
        AuditEvent::BlobDelete,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({ "blob_id": blob_id })),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
