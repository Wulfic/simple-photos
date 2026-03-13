//! Encrypted blob storage endpoints.
//!
//! Handles upload (with SHA-256 integrity check and per-user quota
//! enforcement), paginated listing, streaming download with HTTP
//! Range-request support (byte serving + ETag), and deletion with
//! audit logging.

use axum::body::Body;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
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
pub async fn upload(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<BlobUploadResponse>), AppError> {
    let blob_type = headers
        .get("x-blob-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("photo")
        .to_string();

    tracing::info!(
        user_id = %auth.user_id,
        blob_type = %blob_type,
        body_size = body.len(),
        "Blob upload started"
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

    let size = body.len() as i64;

    if size == 0 {
        tracing::warn!(user_id = %auth.user_id, "Blob upload rejected: empty body");
        return Err(AppError::BadRequest("Empty blob body".into()));
    }

    if size > state.config.storage.max_blob_size_bytes as i64 {
        tracing::warn!(
            user_id = %auth.user_id,
            size = size,
            max = state.config.storage.max_blob_size_bytes,
            "Blob upload rejected: payload too large"
        );
        return Err(AppError::PayloadTooLarge);
    }

    // ── Server-side integrity check ─────────────────────────────────────────
    // If the client sent X-Client-Hash, compute SHA-256 of the body and compare.
    // This catches silent corruption during transit or from proxy/CDN issues.
    if let Some(ref expected_hash) = client_hash {
        let computed_hash = hex::encode(Sha256::digest(&body));
        if computed_hash != *expected_hash {
            tracing::warn!(
                user_id = auth.user_id,
                expected = expected_hash,
                computed = computed_hash,
                "Blob integrity check failed — hash mismatch"
            );
            return Err(AppError::BadRequest(
                "Blob integrity check failed: X-Client-Hash does not match uploaded data".into(),
            ));
        }
    }

    // Check quota
    let used: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM blobs WHERE user_id = ?")
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    let quota: i64 =
        sqlx::query_scalar("SELECT storage_quota_bytes FROM users WHERE id = ?")
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    if quota > 0 && used + size > quota {
        return Err(AppError::Forbidden("Storage quota exceeded".into()));
    }

    let blob_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let storage_root = state.storage_root.read().await.clone();
    let storage_path =
        storage::write_blob(&storage_root, &auth.user_id, &blob_id, &body).await?;

    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .bind(&blob_type)
    .bind(size)
    .bind(&client_hash)
    .bind(&now)
    .bind(&storage_path)
    .bind(&content_hash)
    .execute(&state.pool)
    .await?;

    audit::log(
        &state.pool,
        AuditEvent::BlobUpload,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "blob_id": blob_id,
            "blob_type": blob_type,
            "size_bytes": size
        })),
    )
    .await;

    tracing::info!(
        user_id = %auth.user_id,
        blob_id = %blob_id,
        blob_type = %blob_type,
        size_bytes = size,
        "Blob upload completed successfully"
    );

    Ok((
        StatusCode::CREATED,
        Json(BlobUploadResponse {
            blob_id,
            upload_time: now,
            size,
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
                 ORDER BY upload_time ASC LIMIT ?",
            )
            .bind(&auth.user_id)
            .bind(blob_type)
            .bind(after)
            .bind(limit + 1)
            .fetch_all(&state.pool)
            .await?
        } else {
            sqlx::query_as::<_, BlobRecord>(
                "SELECT id, blob_type, size_bytes, client_hash, upload_time, content_hash FROM blobs \
                 WHERE user_id = ? AND blob_type = ? \
                 ORDER BY upload_time ASC LIMIT ?",
            )
            .bind(&auth.user_id)
            .bind(blob_type)
            .bind(limit + 1)
            .fetch_all(&state.pool)
            .await?
        }
    } else if let Some(ref after) = params.after {
        sqlx::query_as::<_, BlobRecord>(
            "SELECT id, blob_type, size_bytes, client_hash, upload_time, content_hash FROM blobs \
             WHERE user_id = ? AND upload_time > ? \
             ORDER BY upload_time ASC LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(after)
        .bind(limit + 1)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as::<_, BlobRecord>(
            "SELECT id, blob_type, size_bytes, client_hash, upload_time, content_hash FROM blobs \
             WHERE user_id = ? \
             ORDER BY upload_time ASC LIMIT ?",
        )
        .bind(&auth.user_id)
        .bind(limit + 1)
        .fetch_all(&state.pool)
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

/// Stream a blob from disk. Uses tokio ReaderStream so memory usage stays flat
/// regardless of file size (important for large video blobs).
///
/// Supports HTTP Range requests (`Range: bytes=START-END`) for video seeking
/// and download resumption. Returns 206 Partial Content for valid ranges,
/// 416 Range Not Satisfiable for invalid ranges, and 200 OK for full downloads.
pub async fn download(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(blob_id): Path<String>,
) -> Result<Response, AppError> {
    // Validate blob_id format (UUID v4) to prevent path traversal
    if Uuid::parse_str(&blob_id).is_err() {
        return Err(AppError::BadRequest("Invalid blob ID format".into()));
    }

    let (storage_path, _blob_type, size_bytes) = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT storage_path, blob_type, size_bytes FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    tracing::info!(
        blob_id = %blob_id,
        blob_type = %_blob_type,
        size_bytes = size_bytes,
        "[DIAG:BLOB_DL] Blob download requested"
    );

    // Prevent path traversal: storage_path must not contain ".." or absolute paths
    if storage_path.contains("..") || std::path::Path::new(&storage_path).is_absolute() {
        tracing::error!(
            blob_id = blob_id,
            storage_path = storage_path,
            "Suspicious storage path detected — possible path traversal"
        );
        return Err(AppError::Internal("Invalid storage path".into()));
    }

    let storage_root = state.storage_root.read().await.clone();
    let path = storage_root.join(&storage_path);
    let total_size = size_bytes as u64;

    // ── Parse Range header ─────────────────────────────────────────────────
    if let Some(range_header) = headers.get("range").and_then(|v| v.to_str().ok()) {
        if let Some((start, end)) = parse_range_header(range_header, total_size) {
            let length = end - start + 1;

            let mut file = tokio::fs::File::open(&path).await.map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => AppError::NotFound,
                _ => AppError::Internal(format!("Failed to open blob: {}", e)),
            })?;

            // Seek to the requested start position
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| AppError::Internal(format!("Failed to seek: {}", e)))?;

            let stream = tokio_util::io::ReaderStream::new(file.take(length));
            let body = Body::from_stream(stream);

            return Ok(Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Type", HeaderValue::from_static("application/octet-stream"))
                .header("Content-Length", HeaderValue::from(length))
                .header("Content-Range", HeaderValue::from_str(
                    &format!("bytes {}-{}/{}", start, end, total_size)
                ).map_err(|e| AppError::Internal(format!("Invalid Content-Range header: {}", e)))?)
                .header("Accept-Ranges", HeaderValue::from_static("bytes"))
                .header("Cache-Control", HeaderValue::from_static("private, max-age=31536000, immutable"))
                .header("ETag", HeaderValue::from_str(&format!("\"{}\"", blob_id))
                    .map_err(|e| AppError::Internal(format!("Invalid ETag header: {}", e)))?)
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()))?);
        } else {
            // Invalid range → 416 Range Not Satisfiable
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("Content-Range", HeaderValue::from_str(
                    &format!("bytes */{}", total_size)
                ).map_err(|e| AppError::Internal(format!("Invalid Content-Range header: {}", e)))?)
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()))?);
        }
    }

    // ── Full download (no Range header) ────────────────────────────────────
    let file = tokio::fs::File::open(&path).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound,
        _ => AppError::Internal(format!("Failed to open blob: {}", e)),
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_static("application/octet-stream"))
        .header("Content-Length", HeaderValue::from(size_bytes))
        .header("Accept-Ranges", HeaderValue::from_static("bytes"))
        // Blobs are immutable (content-addressed by UUID) — cache aggressively
        .header("Cache-Control", HeaderValue::from_static("private, max-age=31536000, immutable"))
        .header("ETag", HeaderValue::from_str(&format!("\"{}\"", blob_id))
            .map_err(|e| AppError::Internal(format!("Invalid ETag header: {}", e)))?)
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// DELETE /api/blobs/:id — delete a blob and its on-disk file. Returns 204 on success.
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(blob_id): Path<String>,
) -> Result<StatusCode, AppError> {
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

    let storage_root = state.storage_root.read().await.clone();
    storage::delete_blob(&storage_root, &storage_path).await?;

    sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
        .bind(&blob_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state.pool,
        AuditEvent::BlobDelete,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({ "blob_id": blob_id })),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/blobs/:id/thumb — serve the encrypted thumbnail blob associated
/// with a photo blob. Given a photo's `encrypted_blob_id`, looks up the linked
/// `encrypted_thumb_blob_id` in the photos table and streams the thumbnail blob.
///
/// This is a convenience endpoint that frees clients from tracking thumbnail
/// blob IDs separately. Returns 404 if the photo has no thumbnail blob.
pub async fn download_thumb(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(blob_id): Path<String>,
) -> Result<Response, AppError> {
    // Validate blob_id format
    if Uuid::parse_str(&blob_id).is_err() {
        return Err(AppError::BadRequest("Invalid blob ID format".into()));
    }

    // Look up the photo's thumbnail blob ID via the photos table.
    // The photo is identified by its `encrypted_blob_id` matching the given blob_id.
    let thumb_blob_id: Option<String> = sqlx::query_scalar(
        "SELECT encrypted_thumb_blob_id FROM photos \
         WHERE encrypted_blob_id = ? AND user_id = ?",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?; // No photo found with this blob_id

    let thumb_blob_id = thumb_blob_id.ok_or(AppError::NotFound)?; // Photo has no thumbnail blob

    // Find the thumbnail blob's on-disk path
    let (storage_path, size_bytes): (String, i64) = sqlx::query_as(
        "SELECT storage_path, size_bytes FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&thumb_blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    // Path traversal guard
    if storage_path.contains("..") || std::path::Path::new(&storage_path).is_absolute() {
        return Err(AppError::Internal("Invalid storage path".into()));
    }

    let storage_root = state.storage_root.read().await.clone();
    let path = storage_root.join(&storage_path);

    let file = tokio::fs::File::open(&path).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound,
        _ => AppError::Internal(format!("Failed to open thumbnail blob: {}", e)),
    })?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_static("application/octet-stream"))
        .header("Content-Length", HeaderValue::from(size_bytes))
        .header("Cache-Control", HeaderValue::from_static("private, max-age=31536000, immutable"))
        .header("ETag", HeaderValue::from_str(&format!("\"{}\"", thumb_blob_id))
            .map_err(|e| AppError::Internal(format!("Invalid ETag header: {}", e)))?)
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// Parse an HTTP `Range: bytes=START-END` header.
///
/// Supports formats:
/// - `bytes=0-499`     → first 500 bytes
/// - `bytes=500-`      → from byte 500 to the end
/// - `bytes=-500`      → last 500 bytes
///
/// Returns `Some((start, end))` inclusive on success, `None` if invalid.
pub(crate) fn parse_range_header(header: &str, total_size: u64) -> Option<(u64, u64)> {
    let header = header.trim();
    if !header.starts_with("bytes=") {
        return None;
    }
    let range_spec = &header[6..];

    // We only handle single ranges (no multipart)
    if range_spec.contains(',') {
        return None;
    }

    let parts: Vec<&str> = range_spec.splitn(2, '-').collect();
    if parts.len() != 2 {
        return None;
    }

    let (start_str, end_str) = (parts[0].trim(), parts[1].trim());

    if start_str.is_empty() {
        // Suffix range: bytes=-500 → last 500 bytes
        let suffix_len: u64 = end_str.parse().ok()?;
        if suffix_len == 0 || suffix_len > total_size {
            return None;
        }
        let start = total_size - suffix_len;
        Some((start, total_size - 1))
    } else {
        let start: u64 = start_str.parse().ok()?;
        if start >= total_size {
            return None;
        }
        let end = if end_str.is_empty() {
            total_size - 1
        } else {
            let e: u64 = end_str.parse().ok()?;
            e.min(total_size - 1)
        };
        if start > end {
            return None;
        }
        Some((start, end))
    }
}
