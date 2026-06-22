//! File serving endpoints for photos: originals, thumbnails, and web previews.
//!
//! Supports HTTP Range requests (video seeking, resumable downloads) and
//! ETag-based caching (304 Not Modified).

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use base64::Engine as _;

use crate::auth::middleware::AuthUser;
use crate::blobs::storage;
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
        if let Some((start, end)) = crate::http_utils::parse_range_header(range_header, total_size)
        {
            let length = end - start + 1;
            let mut file = tokio::fs::File::open(path)
                .await
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => AppError::NotFound,
                    _ => AppError::Internal(format!("Failed to open file: {e}")),
                })?;

            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| AppError::Internal(format!("Failed to seek: {e}")))?;

            let stream =
                tokio_util::io::ReaderStream::with_capacity(file.take(length), STREAM_BUF_SIZE);
            let body = Body::from_stream(stream);

            let mut builder = Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Type", ct)
                .header("Content-Length", HeaderValue::from(length))
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes {start}-{end}/{total_size}"))
                        .map_err(|e| AppError::Internal(format!("Invalid header: {e}")))?,
                )
                .header("Accept-Ranges", HeaderValue::from_static("bytes"))
                .header(
                    "Cache-Control",
                    HeaderValue::from_static("private, max-age=86400"),
                );
            if let Some(ref ev) = etag_hv {
                builder = builder.header("ETag", ev.clone());
            }
            return builder
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()));
        } else {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes */{total_size}"))
                        .map_err(|e| AppError::Internal(format!("Invalid header: {e}")))?,
                )
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()));
        }
    }

    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound,
            _ => AppError::Internal(format!("Failed to open file: {e}")),
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
    builder
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))
}

// ── Photo Serving Endpoints ──────────────────────────────────────────────────

/// GET /api/photos/:id/file
/// Serve the **original** photo/video/audio file from disk.
/// Supports HTTP Range requests for video seeking and download resumption.
/// Returns ETag for caching; responds with 304 Not Modified on cache hit.
pub async fn serve_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    gallery_token: crate::gallery::access::GalleryToken,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let (file_path, mime_type, size_bytes, enc_blob_id): (String, String, i64, String) =
        sqlx::query_as(
            "SELECT file_path, mime_type, size_bytes, COALESCE(encrypted_blob_id, '') \
             FROM photos WHERE id = ? AND user_id = ?",
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

    // Secure-album gate: if this photo lives in a secure gallery, require a
    // valid unlock token in addition to the account session.
    crate::gallery::access::require_secure_access(&state, &auth.user_id, &photo_id, &gallery_token)
        .await?;

    // ── Encrypted blob fallback (blob-only photos, e.g. rendered duplicates) ─
    if file_path.is_empty() {
        if enc_blob_id.is_empty() {
            return Err(AppError::NotFound);
        }
        let storage_root = (**state.storage_root.load()).clone();
        let key = crate::crypto::load_wrapped_key(&state.pool, &state.config.auth.jwt_secret)
            .await
            .map_err(|e| AppError::Internal(format!("Key load: {e}")))?
            .ok_or_else(|| AppError::Internal("No encryption key configured".into()))?;
        let (blob_storage_path,): (String,) =
            sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
                .bind(&enc_blob_id)
                .bind(&auth.user_id)
                .fetch_optional(&state.read_pool)
                .await?
                .ok_or(AppError::NotFound)?;
        let enc_data = storage::read_blob(&storage_root, &blob_storage_path).await?;
        // Format-aware: handles both the legacy monolithic envelope and the v2
        // chunked container (large videos) — see blobs/chunked.rs.
        let raw_bytes = tokio::task::spawn_blocking(move || {
            crate::blobs::chunked::decrypt_photo_blob(&key, &enc_data)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Decrypt panicked: {e}")))?
        .map_err(|e| AppError::Internal(format!("Decrypt failed: {e}")))?;
        let etag = format!("\"{}-enc-{}\"", photo_id, raw_bytes.len());
        if let Some(not_modified) = check_etag(&headers, &etag) {
            return Ok(not_modified);
        }
        let ct = HeaderValue::from_str(&mime_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream"));
        let len = raw_bytes.len();
        return Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", ct)
            .header("Content-Length", HeaderValue::from(len))
            .header("Accept-Ranges", HeaderValue::from_static("bytes"))
            .header(
                "Cache-Control",
                HeaderValue::from_static("private, max-age=86400"),
            )
            .header(
                "ETag",
                HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")),
            )
            .body(Body::from(raw_bytes))
            .map_err(|e| AppError::Internal(e.to_string()));
    }

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
                _ => AppError::Internal(format!("Failed to open photo: {e}")),
            }
        })
    };

    // ── ETag / conditional response ─────────────────────────────────────
    let etag = format!("\"{photo_id}-{total_size}\"");
    if let Some(not_modified) = check_etag(&headers, &etag) {
        return Ok(not_modified);
    }

    // ── HTTP Range support ─────────────────────────────────────────────
    if let Some(range_header) = headers.get("range").and_then(|v| v.to_str().ok()) {
        if let Some((start, end)) = crate::http_utils::parse_range_header(range_header, total_size)
        {
            let length = end - start + 1;
            let mut file = open_file().await?;

            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| AppError::Internal(format!("Failed to seek: {e}")))?;

            let stream =
                tokio_util::io::ReaderStream::with_capacity(file.take(length), STREAM_BUF_SIZE);
            let body = Body::from_stream(stream);

            return Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Type", content_type)
                .header("Content-Length", HeaderValue::from(length))
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes {start}-{end}/{total_size}"))
                        .map_err(|e| AppError::Internal(format!("Invalid header: {e}")))?,
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
                .map_err(|e| AppError::Internal(e.to_string()));
        } else {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(
                    "Content-Range",
                    HeaderValue::from_str(&format!("bytes */{total_size}"))
                        .map_err(|e| AppError::Internal(format!("Invalid header: {e}")))?,
                )
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()));
        }
    }

    // ── Full download ──────────────────────────────────────────────────
    let file = open_file().await?;
    let stream = tokio_util::io::ReaderStream::with_capacity(file, STREAM_BUF_SIZE);
    let body = Body::from_stream(stream);

    Response::builder()
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
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// GET /api/photos/:id/thumb
/// Serve the thumbnail for a photo.
/// Returns ETag for caching; responds with 304 Not Modified on cache hit.
pub async fn serve_thumbnail(
    State(state): State<AppState>,
    auth: AuthUser,
    gallery_token: crate::gallery::access::GalleryToken,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let (thumb_path_opt, enc_thumb_blob_id): (Option<String>, String) = sqlx::query_as(
        "SELECT thumb_path, COALESCE(encrypted_thumb_blob_id, '') \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or(AppError::NotFound)?;

    // Secure-album gate (see `serve_photo`).
    crate::gallery::access::require_secure_access(&state, &auth.user_id, &photo_id, &gallery_token)
        .await?;

    // ── Encrypted thumbnail fallback (blob-only duplicates) ──────────────────
    if thumb_path_opt.is_none() {
        if enc_thumb_blob_id.is_empty() {
            return Err(AppError::NotFound);
        }
        let storage_root = (**state.storage_root.load()).clone();
        let key = crate::crypto::load_wrapped_key(&state.pool, &state.config.auth.jwt_secret)
            .await
            .map_err(|e| AppError::Internal(format!("Key load: {e}")))?
            .ok_or_else(|| AppError::Internal("No encryption key configured".into()))?;
        let (blob_storage_path,): (String,) =
            sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
                .bind(&enc_thumb_blob_id)
                .bind(&auth.user_id)
                .fetch_optional(&state.read_pool)
                .await?
                .ok_or(AppError::NotFound)?;
        let enc_data = storage::read_blob(&storage_root, &blob_storage_path).await?;
        let plaintext =
            tokio::task::spawn_blocking(move || crate::crypto::decrypt(&key, &enc_data))
                .await
                .map_err(|e| AppError::Internal(format!("Decrypt panicked: {e}")))?
                .map_err(|e| AppError::Internal(format!("Decrypt failed: {e}")))?;
        let envelope: serde_json::Value = serde_json::from_slice(&plaintext)
            .map_err(|e| AppError::Internal(format!("Thumb envelope JSON: {e}")))?;
        let data_b64 = envelope["data"]
            .as_str()
            .ok_or_else(|| AppError::Internal("Missing 'data' in thumb envelope".into()))?;
        let raw_bytes = base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .map_err(|e| AppError::Internal(format!("Base64 decode thumb: {e}")))?;
        let etag = format!("\"{}-enc-thumb-{}\"", photo_id, raw_bytes.len());
        if let Some(not_modified) = check_etag(&headers, &etag) {
            return Ok(not_modified);
        }
        let content_type = if enc_thumb_blob_id.ends_with(".gif") {
            "image/gif"
        } else {
            "image/jpeg"
        };
        let len = raw_bytes.len();
        return Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", HeaderValue::from_static(content_type))
            .header("Content-Length", HeaderValue::from(len))
            .header(
                "Cache-Control",
                HeaderValue::from_static("private, max-age=86400"),
            )
            .header(
                "ETag",
                HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")),
            )
            .body(Body::from(raw_bytes))
            .map_err(|e| AppError::Internal(e.to_string()));
    }

    let thumb_path = thumb_path_opt.ok_or(AppError::NotFound)?;
    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&thumb_path);

    // If thumbnail doesn't exist yet, return 202 Accepted to signal "pending".
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Response::builder()
            .status(StatusCode::ACCEPTED)
            .header("Content-Type", HeaderValue::from_static("application/json"))
            .body(Body::from(r#"{"status":"pending"}"#))
            .map_err(|e| AppError::Internal(e.to_string()));
    }

    let meta = tokio::fs::metadata(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read thumbnail: {e}")))?;

    // ETag for thumbnails — ID + file size on disk
    let etag = format!("\"{}-thumb-{}\"", photo_id, meta.len());
    if let Some(not_modified) = check_etag(&headers, &etag) {
        return Ok(not_modified);
    }

    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to open thumbnail: {e}")))?;

    let stream = tokio_util::io::ReaderStream::with_capacity(file, STREAM_BUF_SIZE);
    let body = Body::from_stream(stream);

    // Determine Content-Type from thumbnail path extension
    let content_type = if full_path.extension().and_then(|e| e.to_str()) == Some("gif") {
        "image/gif"
    } else {
        "image/jpeg"
    };

    Response::builder()
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
        .map_err(|e| AppError::Internal(e.to_string()))
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
    gallery_token: crate::gallery::access::GalleryToken,
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

    // Secure-album gate (see `serve_photo`).
    crate::gallery::access::require_secure_access(&state, &auth.user_id, &photo_id, &gallery_token)
        .await?;

    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&file_path);
    let content_type = mime_type.as_str();

    let etag = format!("\"{photo_id}-orig-{size_bytes}\"");
    serve_file_with_range(
        &full_path,
        size_bytes as u64,
        content_type,
        &headers,
        Some(&etag),
    )
    .await
}

/// GET /api/photos/:id/source-file
/// Serve the **original unconverted** source file for a converted photo.
/// Returns 404 if the photo was not converted or the source file is missing.
pub async fn serve_source_file(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let source_path: Option<String> =
        sqlx::query_scalar("SELECT source_path FROM photos WHERE id = ? AND user_id = ?")
            .bind(&photo_id)
            .bind(&auth.user_id)
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let source_path = source_path.ok_or_else(|| {
        tracing::debug!(
            photo_id = %photo_id,
            "serve_source_file: photo has no source_path (not converted)"
        );
        AppError::NotFound
    })?;

    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&source_path);

    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        tracing::warn!(
            photo_id = %photo_id,
            source_path = %source_path,
            "serve_source_file: original source file not found on disk"
        );
        return Err(AppError::NotFound);
    }

    let meta = tokio::fs::metadata(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read source file: {e}")))?;

    let total_size = meta.len();

    // Guess MIME type from extension
    let content_type = match full_path.extension().and_then(|e| e.to_str()) {
        Some("heic" | "heif") => "image/heic",
        Some("mkv") => "video/x-matroska",
        Some("avi") => "video/x-msvideo",
        Some("wmv") => "video/x-ms-wmv",
        Some("tiff" | "tif") => "image/tiff",
        Some("bmp") => "image/bmp",
        Some("webm") => "video/webm",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        _ => "application/octet-stream",
    };

    let etag = format!("\"{photo_id}-source-{total_size}\"");

    // Force download via Content-Disposition
    let filename = full_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("original");
    let disposition = format!("attachment; filename=\"{filename}\"");

    let mut resp =
        serve_file_with_range(&full_path, total_size, content_type, &headers, Some(&etag)).await?;

    resp.headers_mut().insert(
        "Content-Disposition",
        HeaderValue::from_str(&disposition).unwrap_or(HeaderValue::from_static("attachment")),
    );

    Ok(resp)
}

/// GET /api/photos/{id}/motion-video
/// Serve the embedded MP4 video extracted from a motion photo.
///
/// For photos with `motion_video_blob_id` set, serves the blob.
/// Otherwise, extracts the video trailer on-the-fly from the JPEG
/// using the XMP-specified offset.
pub async fn serve_motion_video(
    State(state): State<AppState>,
    auth: AuthUser,
    gallery_token: crate::gallery::access::GalleryToken,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    // Check that the photo exists, belongs to user, and is a motion photo
    let row: Option<(String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT file_path, motion_video_blob_id, photo_subtype, encrypted_blob_id \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?;

    let (file_path, motion_blob_id, subtype, enc_blob_id) = row.ok_or(AppError::NotFound)?;

    // Secure-album gate (see `serve_photo`).
    crate::gallery::access::require_secure_access(&state, &auth.user_id, &photo_id, &gallery_token)
        .await?;

    if subtype.as_deref() != Some("motion") {
        return Err(AppError::BadRequest(
            "Photo is not a motion photo".to_string(),
        ));
    }

    // If a motion video blob is already stored separately, serve it.
    // Resolve via the blobs table's recorded storage_path — deriving the
    // path by convention broke here before: extraction wrote flat
    // `blobs/{id}.mp4` while this handler guessed the sharded `.bin`
    // layout, so the stored blob was never served.
    if let Some(ref blob_id) = motion_blob_id {
        let storage_root = (**state.storage_root.load()).clone();
        let recorded: Option<(String,)> =
            sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
                .bind(blob_id)
                .bind(&auth.user_id)
                .fetch_optional(&state.read_pool)
                .await?;

        if let Some((storage_path,)) = recorded {
            let blob_path = storage_root.join(&storage_path);
            if tokio::fs::try_exists(&blob_path).await.unwrap_or(false) {
                let data = tokio::fs::read(&blob_path).await.map_err(|e| {
                    AppError::Internal(format!("Failed to read motion video blob: {e}"))
                })?;
                let len = data.len();
                return Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "video/mp4")
                    .header("Content-Length", len)
                    .body(Body::from(data))
                    .map_err(|e| AppError::Internal(format!("Response build: {e}")));
            }
            tracing::warn!(
                photo_id = %photo_id,
                blob_id = %blob_id,
                storage_path = %storage_path,
                "Motion video blob row exists but file is missing — falling back to on-the-fly extraction"
            );
        }
    }

    // Extract on-the-fly from the original JPEG bytes using the XMP offset.
    //
    // For encrypted backups (Android), the photo has no plaintext file on disk
    // (`file_path` is empty) — the bytes live in an encrypted blob. The server
    // holds the wrapped key for serving, so decrypt the photo blob and unwrap
    // the JSON envelope to recover the JPEG, then extract the MP4 trailer just
    // like the plaintext path. No separate motion_video blob is needed.
    let storage_root = (**state.storage_root.load()).clone();
    let data: Vec<u8> = if !file_path.is_empty() {
        let full_path = storage_root.join(&file_path);
        tokio::fs::read(&full_path).await.map_err(|e| {
            AppError::Internal(format!("Failed to read photo file for motion video: {e}"))
        })?
    } else if let Some(blob_id) = enc_blob_id.filter(|s| !s.is_empty()) {
        let key = crate::crypto::load_wrapped_key(&state.pool, &state.config.auth.jwt_secret)
            .await
            .map_err(|e| AppError::Internal(format!("Key load: {e}")))?
            .ok_or_else(|| AppError::Internal("No encryption key configured".into()))?;
        let (blob_storage_path,): (String,) =
            sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
                .bind(&blob_id)
                .bind(&auth.user_id)
                .fetch_optional(&state.read_pool)
                .await?
                .ok_or(AppError::NotFound)?;
        let enc_data = storage::read_blob(&storage_root, &blob_storage_path).await?;
        let plaintext =
            tokio::task::spawn_blocking(move || crate::crypto::decrypt(&key, &enc_data))
                .await
                .map_err(|e| AppError::Internal(format!("Decrypt panicked: {e}")))?
                .map_err(|e| AppError::Internal(format!("Decrypt failed: {e}")))?;
        let envelope: serde_json::Value = serde_json::from_slice(&plaintext)
            .map_err(|e| AppError::Internal(format!("Blob envelope JSON: {e}")))?;
        let data_b64 = envelope["data"]
            .as_str()
            .ok_or_else(|| AppError::Internal("Missing 'data' field in blob envelope".into()))?;
        base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .map_err(|e| AppError::Internal(format!("Base64 decode: {e}")))?
    } else {
        return Err(AppError::NotFound);
    };

    // Offset resolution mirrors `motion::extract_and_store_motion_video`:
    // Pixel/Google declare it in XMP; Samsung carries no XMP offset and instead
    // ends with a `MotionPhoto_Data` SEF trailer located by a byte scan.
    let subtype_info = super::metadata::extract_xmp_subtype(&data);
    let offset = subtype_info
        .motion_video_offset
        .or_else(|| super::motion::find_samsung_motion_offset(&data))
        .ok_or_else(|| {
            AppError::BadRequest(
                "Motion video offset not found (no XMP offset, no Samsung trailer)".to_string(),
            )
        })?;

    let video_bytes = super::metadata::extract_motion_video(&data, offset).ok_or_else(|| {
        AppError::Internal("Failed to extract motion video from JPEG".to_string())
    })?;

    let len = video_bytes.len();
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "video/mp4")
        .header("Content-Length", len)
        .body(Body::from(video_bytes))
        .map_err(|e| AppError::Internal(format!("Response build: {e}")))
}
