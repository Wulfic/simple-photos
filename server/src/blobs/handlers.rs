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
    "thumbnail",
    "video_thumbnail",
    "album_manifest",
];

#[derive(Debug, Deserialize)]
pub struct ListBlobsQuery {
    pub blob_type: Option<String>,
    pub after: Option<String>,
    pub limit: Option<i64>,
}

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
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .bind(&blob_type)
    .bind(size)
    .bind(&client_hash)
    .bind(&now)
    .bind(&storage_path)
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
                "SELECT id, blob_type, size_bytes, client_hash, upload_time FROM blobs \
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
                "SELECT id, blob_type, size_bytes, client_hash, upload_time FROM blobs \
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
            "SELECT id, blob_type, size_bytes, client_hash, upload_time FROM blobs \
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
            "SELECT id, blob_type, size_bytes, client_hash, upload_time FROM blobs \
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

    // Prevent path traversal: storage_path must not contain ".." or absolute paths
    if storage_path.contains("..") || storage_path.starts_with('/') {
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
                ).unwrap())
                .header("Accept-Ranges", HeaderValue::from_static("bytes"))
                .header("Cache-Control", HeaderValue::from_static("no-store"))
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()))?);
        } else {
            // Invalid range → 416 Range Not Satisfiable
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("Content-Range", HeaderValue::from_str(
                    &format!("bytes */{}", total_size)
                ).unwrap())
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
        // Prevent browsers from caching decrypted blobs
        .header("Cache-Control", HeaderValue::from_static("no-store"))
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

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

/// Parse an HTTP `Range: bytes=START-END` header.
///
/// Supports formats:
/// - `bytes=0-499`     → first 500 bytes
/// - `bytes=500-`      → from byte 500 to the end
/// - `bytes=-500`      → last 500 bytes
///
/// Returns `Some((start, end))` inclusive on success, `None` if invalid.
fn parse_range_header(header: &str, total_size: u64) -> Option<(u64, u64)> {
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
