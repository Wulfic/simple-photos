//! File serving endpoints for photos: originals, thumbnails, and web previews.
//!
//! Supports HTTP Range requests (video seeking, resumable downloads) and
//! ETag-based caching (304 Not Modified).

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// Stream buffer size for file serving — 64 KB per chunk instead of the
/// default 4 KB.  Larger chunks reduce the number of syscalls and context
/// switches, which is critical when serving large video files or many
/// thumbnails concurrently.
pub(crate) const STREAM_BUF_SIZE: usize = 64 * 1024;

/// Check `If-None-Match` header against our ETag.  Returns `Some(304)` if
/// the client already has the current version.
pub(crate) fn check_etag(headers: &HeaderMap, etag: &str) -> Option<Response> {
    if let Some(inm) = headers.get("if-none-match").and_then(|v| v.to_str().ok()) {
        if inm == etag || inm.trim_matches('"') == etag.trim_matches('"') {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(
                    "ETag",
                    HeaderValue::from_str(etag).unwrap_or(HeaderValue::from_static("")),
                )
                .header(
                    "Cache-Control",
                    HeaderValue::from_static("private, max-age=86400"),
                )
                .body(Body::empty())
                .ok();
        }
    }
    None
}

/// Internal helper: serve a file with optional HTTP Range + ETag support.
/// `etag` is optional — if provided, the response includes the ETag header
/// and If-None-Match is checked for 304 early-return.
pub(crate) async fn serve_file_with_range(
    path: &std::path::Path,
    total_size: u64,
    content_type: &str,
    headers: &HeaderMap,
    etag: Option<&str>,
) -> Result<Response, AppError> {
    let ct = HeaderValue::from_str(content_type)
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));

    // ETag conditional check
    if let Some(tag) = etag {
        if let Some(not_modified) = check_etag(headers, tag) {
            return Ok(not_modified);
        }
    }

    let etag_hv = etag.and_then(|t| HeaderValue::from_str(t).ok());

    if let Some(range_header) = headers.get("range").and_then(|v| v.to_str().ok()) {
        if let Some((start, end)) =
            crate::http_utils::parse_range_header(range_header, total_size)
        {
            let length = end - start + 1;
            let mut file = tokio::fs::File::open(path)
                .await
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => AppError::NotFound,
                    _ => AppError::Internal(format!("Failed to open file: {}", e)),
                })?;

            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| AppError::Internal(format!("Failed to seek: {}", e)))?;

            let stream =
                tokio_util::io::ReaderStream::with_capacity(file.take(length), STREAM_BUF_SIZE);
            let body = Body::from_stream(stream);

            let mut builder = Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Type", ct)
                .header("Content-Length", HeaderValue::from(length))
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes {}-{}/{}", start, end, total_size))
                        .map_err(|e| AppError::Internal(format!("Invalid header: {}", e)))?,
                )
                .header("Accept-Ranges", HeaderValue::from_static("bytes"))
                .header(
                    "Cache-Control",
                    HeaderValue::from_static("private, max-age=86400"),
                );
            if let Some(ref ev) = etag_hv {
                builder = builder.header("ETag", ev.clone());
            }
            return Ok(builder
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()))?);
        } else {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes */{}", total_size))
                        .map_err(|e| AppError::Internal(format!("Invalid header: {}", e)))?,
                )
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()))?);
        }
    }

    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open file: {}", e)),
        })?;

    let stream = tokio_util::io::ReaderStream::with_capacity(file, STREAM_BUF_SIZE);
    let body = Body::from_stream(stream);

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", ct)
        .header("Content-Length", HeaderValue::from(total_size))
        .header("Accept-Ranges", HeaderValue::from_static("bytes"))
        .header(
            "Cache-Control",
            HeaderValue::from_static("private, max-age=86400"),
        );
    if let Some(ref ev) = etag_hv {
        builder = builder.header("ETag", ev.clone());
    }
    Ok(builder
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

// ── Photo Serving Endpoints ──────────────────────────────────────────────────

/// GET /api/photos/:id/file
/// Serve the **original** photo/video/audio file from disk.
/// Supports HTTP Range requests for video seeking and download resumption.
/// Returns ETag for caching; responds with 304 Not Modified on cache hit.
pub async fn serve_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let (file_path, mime_type, size_bytes): (String, String, i64) = sqlx::query_as(
        "SELECT file_path, mime_type, size_bytes FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or_else(|| {
        tracing::warn!(
            user_id = %auth.user_id,
            photo_id = %photo_id,
            "serve_photo: photo not found in database"
        );
        AppError::NotFound
    })?;

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&file_path);

    tracing::debug!(
        user_id = %auth.user_id,
        photo_id = %photo_id,
        file_path = %file_path,
        full_path = %full_path.display(),
        size_bytes = size_bytes,
        "serve_photo: serving file"
    );

    let total_size = size_bytes as u64;
    let content_type = HeaderValue::from_str(&mime_type)
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));

    let open_file = || async {
        tokio::fs::File::open(&full_path).await.map_err(|e| {
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
        })
    };

    // ── ETag / conditional response ─────────────────────────────────────
    let etag = format!("\"{}-{}\"", photo_id, total_size);
    if let Some(not_modified) = check_etag(&headers, &etag) {
        return Ok(not_modified);
    }

    // ── HTTP Range support ─────────────────────────────────────────────
    if let Some(range_header) = headers.get("range").and_then(|v| v.to_str().ok()) {
        if let Some((start, end)) =
            crate::http_utils::parse_range_header(range_header, total_size)
        {
            let length = end - start + 1;
            let mut file = open_file().await?;

            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| AppError::Internal(format!("Failed to seek: {}", e)))?;

            let stream =
                tokio_util::io::ReaderStream::with_capacity(file.take(length), STREAM_BUF_SIZE);
            let body = Body::from_stream(stream);

            return Ok(Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Type", content_type)
                .header("Content-Length", HeaderValue::from(length))
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes {}-{}/{}", start, end, total_size))
                        .map_err(|e| AppError::Internal(format!("Invalid header: {}", e)))?,
                )
                .header("Accept-Ranges", HeaderValue::from_static("bytes"))
                .header(
                    "ETag",
                    HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")),
                )
                .header(
                    "Cache-Control",
                    HeaderValue::from_static("private, max-age=86400"),
                )
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()))?);
        } else {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes */{}", total_size))
                        .map_err(|e| AppError::Internal(format!("Invalid header: {}", e)))?,
                )
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()))?);
        }
    }

    // ── Full download ──────────────────────────────────────────────────
    let file = open_file().await?;
    let stream = tokio_util::io::ReaderStream::with_capacity(file, STREAM_BUF_SIZE);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", content_type)
        .header("Content-Length", HeaderValue::from(size_bytes))
        .header("Accept-Ranges", HeaderValue::from_static("bytes"))
        .header(
            "ETag",
            HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")),
        )
        .header(
            "Cache-Control",
            HeaderValue::from_static("private, max-age=86400"),
        )
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// GET /api/photos/:id/thumb
/// Serve the thumbnail for a photo.
/// Returns ETag for caching; responds with 304 Not Modified on cache hit.
pub async fn serve_thumbnail(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let thumb_path: Option<String> =
        sqlx::query_scalar("SELECT thumb_path FROM photos WHERE id = ? AND user_id = ?")
            .bind(&photo_id)
            .bind(&auth.user_id)
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;
    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&thumb_path);

    // If thumbnail doesn't exist yet, return 202 Accepted to signal "pending".
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Ok(Response::builder()
            .status(StatusCode::ACCEPTED)
            .header("Content-Type", HeaderValue::from_static("application/json"))
            .body(Body::from(r#"{"status":"pending"}"#))
            .map_err(|e| AppError::Internal(e.to_string()))?);
    }

    let meta = tokio::fs::metadata(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read thumbnail: {}", e)))?;

    // ETag for thumbnails — ID + file size on disk
    let etag = format!("\"{}-thumb-{}\"", photo_id, meta.len());
    if let Some(not_modified) = check_etag(&headers, &etag) {
        return Ok(not_modified);
    }

    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to open thumbnail: {}", e)))?;

    let stream = tokio_util::io::ReaderStream::with_capacity(file, STREAM_BUF_SIZE);
    let body = Body::from_stream(stream);

    // Determine Content-Type from thumbnail path extension
    let content_type = if full_path.extension().and_then(|e| e.to_str()) == Some("gif") {
        "image/gif"
    } else {
        "image/jpeg"
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_static(content_type))
        .header("Content-Length", HeaderValue::from(meta.len()))
        .header(
            "ETag",
            HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")),
        )
        .header(
            "Cache-Control",
            HeaderValue::from_static("private, max-age=86400"),
        )
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// GET /api/photos/:id/web
/// Serve a browser-compatible version of the media.
///
/// Since all supported formats are browser-native, this simply serves
/// the original file directly (equivalent to `/photos/:id/file`).
///
/// Supports HTTP Range requests for video seeking.
pub async fn serve_web(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let (file_path, mime_type, _filename, size_bytes): (String, String, String, i64) = sqlx::query_as(
        "SELECT file_path, mime_type, filename, size_bytes FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&file_path);
    let content_type = mime_type.as_str();

    let etag = format!("\"{}-orig-{}\"", photo_id, size_bytes);
    serve_file_with_range(
        &full_path,
        size_bytes as u64,
        content_type,
        &headers,
        Some(&etag),
    )
    .await
}
