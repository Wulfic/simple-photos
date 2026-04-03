//! Blob download (streaming) endpoints.
//!
//! Streams encrypted blobs from disk with HTTP Range-request support
//! (byte serving + ETag).  Memory usage stays flat regardless of file
//! size — important for multi-gigabyte video blobs.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// GET /api/blobs/{id} — stream an encrypted blob from disk.
///
/// Uses tokio ReaderStream so memory usage stays flat regardless of file size
/// (important for large video blobs).
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
    .fetch_optional(&state.read_pool)
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

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let path = storage_root.join(&storage_path);
    let total_size = size_bytes as u64;

    // ── If-None-Match → 304 (blobs are immutable, ETag = blob_id) ──────
    let etag = format!("\"{}\"", blob_id);
    if let Some(inm) = headers.get("if-none-match").and_then(|v| v.to_str().ok()) {
        if inm == etag || inm.trim_matches('"') == blob_id {
            return Ok(Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(
                    "ETag",
                    HeaderValue::from_str(&etag)
                        .map_err(|e| AppError::Internal(format!("Invalid ETag header: {}", e)))?,
                )
                .header(
                    "Cache-Control",
                    HeaderValue::from_static("private, max-age=31536000, immutable"),
                )
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()))?);
        }
    }

    /// 64 KB stream buffer for blob file serving.
    const BLOB_BUF: usize = 64 * 1024;

    // ── Parse Range header ─────────────────────────────────────────────────
    if let Some(range_header) = headers.get("range").and_then(|v| v.to_str().ok()) {
        if let Some((start, end)) = crate::http_utils::parse_range_header(range_header, total_size) {
            let length = end - start + 1;

            let mut file = tokio::fs::File::open(&path)
                .await
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => AppError::NotFound,
                    _ => AppError::Internal(format!("Failed to open blob: {}", e)),
                })?;

            // Seek to the requested start position
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| AppError::Internal(format!("Failed to seek: {}", e)))?;

            let stream = tokio_util::io::ReaderStream::with_capacity(file.take(length), BLOB_BUF);
            let body = Body::from_stream(stream);

            return Ok(Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(
                    "Content-Type",
                    HeaderValue::from_static("application/octet-stream"),
                )
                .header("Content-Length", HeaderValue::from(length))
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes {}-{}/{}", start, end, total_size))
                        .map_err(|e| {
                            AppError::Internal(format!("Invalid Content-Range header: {}", e))
                        })?,
                )
                .header("Accept-Ranges", HeaderValue::from_static("bytes"))
                .header(
                    "Cache-Control",
                    HeaderValue::from_static("private, max-age=31536000, immutable"),
                )
                .header(
                    "ETag",
                    HeaderValue::from_str(&etag)
                        .map_err(|e| AppError::Internal(format!("Invalid ETag header: {}", e)))?,
                )
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()))?);
        } else {
            // Invalid range → 416 Range Not Satisfiable
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes */{}", total_size)).map_err(|e| {
                        AppError::Internal(format!("Invalid Content-Range header: {}", e))
                    })?,
                )
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()))?);
        }
    }

    // ── Full download (no Range header) ────────────────────────────────────
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open blob: {}", e)),
        })?;

    let stream = tokio_util::io::ReaderStream::with_capacity(file, BLOB_BUF);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            "Content-Type",
            HeaderValue::from_static("application/octet-stream"),
        )
        .header("Content-Length", HeaderValue::from(size_bytes))
        .header("Accept-Ranges", HeaderValue::from_static("bytes"))
        // Blobs are immutable (content-addressed by UUID) — cache aggressively
        .header(
            "Cache-Control",
            HeaderValue::from_static("private, max-age=31536000, immutable"),
        )
        .header(
            "ETag",
            HeaderValue::from_str(&etag)
                .map_err(|e| AppError::Internal(format!("Invalid ETag header: {}", e)))?,
        )
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// GET /api/blobs/:id/thumb — serve the encrypted thumbnail blob associated
/// with a photo blob. Given a photo's `encrypted_blob_id`, looks up the linked
/// `encrypted_thumb_blob_id` in the photos table and streams the thumbnail blob.
///
/// This is a convenience endpoint that frees clients from tracking thumbnail
/// blob IDs separately. Returns 404 if the photo has no thumbnail blob.
///
/// **Performance:** Uses a single JOIN query instead of two sequential queries
/// (photos → blobs). This halves the round-trips to SQLite on every encrypted
/// thumbnail request.
pub async fn download_thumb(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(blob_id): Path<String>,
) -> Result<Response, AppError> {
    // Validate blob_id format
    if Uuid::parse_str(&blob_id).is_err() {
        return Err(AppError::BadRequest("Invalid blob ID format".into()));
    }

    // Single JOIN: look up both the thumbnail blob ID and its storage
    // location in one query instead of two sequential round-trips.
    // photos.encrypted_blob_id → photos.encrypted_thumb_blob_id → blobs row
    let (thumb_blob_id, storage_path, size_bytes): (String, String, i64) = sqlx::query_as(
        "SELECT b.id, b.storage_path, b.size_bytes \
         FROM photos p \
         JOIN blobs b ON b.id = p.encrypted_thumb_blob_id \
         WHERE p.encrypted_blob_id = ? AND p.user_id = ?",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or(AppError::NotFound)?;

    // Path traversal guard
    if storage_path.contains("..") || std::path::Path::new(&storage_path).is_absolute() {
        return Err(AppError::Internal("Invalid storage path".into()));
    }

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let path = storage_root.join(&storage_path);

    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open thumbnail blob: {}", e)),
        })?;

    // ── If-None-Match → 304 ────────────────────────────────────────────
    let etag = format!("\"{}\"", thumb_blob_id);
    if let Some(inm) = headers.get("if-none-match").and_then(|v| v.to_str().ok()) {
        if inm == etag || inm.trim_matches('"') == thumb_blob_id.as_str() {
            return Ok(Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(
                    "ETag",
                    HeaderValue::from_str(&etag)
                        .map_err(|e| AppError::Internal(format!("Invalid ETag header: {}", e)))?,
                )
                .header(
                    "Cache-Control",
                    HeaderValue::from_static("private, max-age=31536000, immutable"),
                )
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()))?);
        }
    }

    let stream = tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            "Content-Type",
            HeaderValue::from_static("application/octet-stream"),
        )
        .header("Content-Length", HeaderValue::from(size_bytes))
        .header(
            "Cache-Control",
            HeaderValue::from_static("private, max-age=31536000, immutable"),
        )
        .header(
            "ETag",
            HeaderValue::from_str(&etag)
                .map_err(|e| AppError::Internal(format!("Invalid ETag header: {}", e)))?,
        )
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}
