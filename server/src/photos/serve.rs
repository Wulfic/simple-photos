//! File serving endpoints for photos: originals, thumbnails, and web previews.
//!
//! Supports HTTP Range requests (video seeking, resumable downloads) and
//! ETag-based caching (304 Not Modified).

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use base64::Engine as _;
use serde::Deserialize;

use crate::auth::middleware::AuthUser;
use crate::blobs::storage;
use crate::error::AppError;
use crate::http_utils::{media_cache_control, Confidentiality, MEDIA_CACHE_1D};
use crate::state::AppState;

/// Stream buffer size for file serving — 64 KB per chunk instead of the
/// default 4 KB.  Larger chunks reduce the number of syscalls and context
/// switches, which is critical when serving large video files or many
/// thumbnails concurrently.
pub(crate) const STREAM_BUF_SIZE: usize = 64 * 1024;

/// Check `If-None-Match` header against our ETag.  Returns `Some(304)` if
/// the client already has the current version.
///
/// `conf` decides the `Cache-Control` on the 304 exactly as it does on the 200.
/// A 304 that said `max-age=86400` for a secure item would *extend* the life of
/// a cache entry the 200 refused to create — the one response where getting this
/// wrong is invisible, because there is no body to notice.
pub(crate) fn check_etag(
    headers: &HeaderMap,
    etag: &str,
    conf: Confidentiality,
) -> Option<Response> {
    if let Some(inm) = headers.get("if-none-match").and_then(|v| v.to_str().ok()) {
        if inm == etag || inm.trim_matches('"') == etag.trim_matches('"') {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(
                    "ETag",
                    HeaderValue::from_str(etag).unwrap_or(HeaderValue::from_static("")),
                )
                .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D))
                .body(Body::empty())
                .ok();
        }
    }
    None
}

/// Cheaply test whether a blob file is a v2 chunked container by reading only
/// its 8-byte magic prefix. Returns `false` on any I/O error (treat as v1).
async fn peek_is_chunked(path: &std::path::Path) -> bool {
    use tokio::io::AsyncReadExt;
    match tokio::fs::File::open(path).await {
        Ok(mut f) => {
            let mut magic = [0u8; 8];
            f.read_exact(&mut magic)
                .await
                .map(|_| crate::blobs::chunked::is_chunked(&magic))
                .unwrap_or(false)
        }
        Err(_) => false,
    }
}

/// Build a streaming response body that decrypts a v2 chunked blob one frame at
/// a time on a blocking thread, so a multi-GB video never lives in memory. A
/// bounded channel provides backpressure between the decrypt thread and the
/// network.
fn chunked_decrypt_body(key: [u8; 32], path: std::path::PathBuf) -> Body {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(4);
    tokio::task::spawn_blocking(move || {
        let res = crate::blobs::chunked::for_each_plaintext_chunk(&key, &path, |chunk| {
            tx.blocking_send(Ok(axum::body::Bytes::from(chunk))).is_ok()
        });
        if let Err(e) = res {
            let _ = tx.blocking_send(Err(std::io::Error::other(e)));
        }
    });
    Body::from_stream(async_stream::stream! {
        while let Some(item) = rx.recv().await {
            yield item;
        }
    })
}

/// Internal helper: serve a file with optional HTTP Range + ETag support.
/// `etag` is optional — if provided, the response includes the ETag header
/// and If-None-Match is checked for 304 early-return.
///
/// `conf` is threaded to all three exit paths (304, 206, 200) rather than
/// applied by the caller afterwards: a `Cache-Control` set on only two of the
/// three is a leak that appears exactly when a client seeks or revalidates.
pub(crate) async fn serve_file_with_range(
    path: &std::path::Path,
    total_size: u64,
    content_type: &str,
    headers: &HeaderMap,
    etag: Option<&str>,
    conf: Confidentiality,
) -> Result<Response, AppError> {
    let ct = HeaderValue::from_str(content_type)
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));

    // ETag conditional check
    if let Some(tag) = etag {
        if let Some(not_modified) = check_etag(headers, tag, conf) {
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

            let mut builder =
                crate::http_utils::partial_content_builder(ct, start, end, total_size)?
                    .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D));
            if let Some(ref ev) = etag_hv {
                builder = builder.header("ETag", ev.clone());
            }
            return builder
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()));
        } else {
            return crate::http_utils::range_not_satisfiable(total_size);
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
        .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D));
    if let Some(ref ev) = etag_hv {
        builder = builder.header("ETag", ev.clone());
    }
    builder
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))
}

// ── Photo Serving Endpoints ──────────────────────────────────────────────────

/// Where [`serve_photo`] should read bytes from, once a rendition selector has
/// been resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServeTarget {
    /// Storage-root-relative path, or empty to mean "read the encrypted blob".
    /// Empty is a **sentinel** in this handler, not a missing value.
    pub file_path: String,
    /// Encrypted blob id, or empty when serving a plaintext file.
    pub enc_blob_id: String,
    pub size_bytes: i64,
    /// Cache identity. Must distinguish rungs from each other and from the
    /// original, or a client is handed a cached copy of the wrong quality.
    pub etag_id: String,
    /// **The rung's** type, not the parent photo's. The ladder always encodes
    /// H.264 in MP4 (`rung_generate::store_encrypted` stamps `video/mp4` on the
    /// envelope, `store_plaintext` writes a `.mp4`), but the source it was made
    /// from may be a `.mov` — and 10 of the live library's videos are. Serving a
    /// downscale under the source's `video/quicktime` hands the player bytes
    /// that do not match the type it was promised.
    pub content_type: String,
}

/// What the ladder produces, always. Kept next to the target so the coupling to
/// `rung_generate`'s output format is visible from the serve side.
const RENDITION_MIME: &str = "video/mp4";

/// Resolve one rung of the ladder to the bytes that back it.
///
/// Pure, and separated from the handler for the same reason
/// `ladder::rung_dimensions` is: the interesting part is a small mapping whose
/// mistakes are silent (serving the original for a 1080p request, or two rungs
/// sharing an ETag), and burying it in a 300-line streaming handler would make
/// it verifiable only against a live server with a generated ladder.
pub(crate) fn rendition_serve_target(
    photo_id: &str,
    rung: &crate::transcode::renditions::StoredRendition,
) -> ServeTarget {
    // `is_playable` guarantees at least one locator; blob wins if a row ever
    // carries both, because that is the mode both clients actually play from.
    let (file_path, enc_blob_id) = match (&rung.blob_id, &rung.file_path) {
        (Some(blob), _) => (String::new(), blob.clone()),
        (None, Some(path)) => (path.clone(), String::new()),
        // Unreachable through `list_renditions`, which filters these out. Serve
        // nothing rather than silently falling back to the original: a client
        // that asked for 1080p and got 4K "works" while defeating the point.
        (None, None) => (String::new(), String::new()),
    };

    ServeTarget {
        file_path,
        enc_blob_id,
        size_bytes: rung.size_bytes,
        etag_id: format!("{photo_id}.r{}", rung.short_edge),
        content_type: RENDITION_MIME.to_string(),
    }
}

/// Query parameters for [`serve_photo`].
#[derive(Debug, Deserialize)]
pub struct ServePhotoQuery {
    /// Video quality ladder selector (#49): the `short_edge` of the rung to
    /// serve instead of the original. Absent = the original, which is the
    /// behaviour every existing caller relies on.
    pub rendition: Option<i64>,
}

/// GET /api/photos/:id/file
/// Serve the **original** photo/video/audio file from disk — or, with
/// `?rendition=<short_edge>`, one rung of its quality ladder (#49).
/// Supports HTTP Range requests for video seeking and download resumption.
/// Returns ETag for caching; responds with 304 Not Modified on cache hit.
pub async fn serve_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    gallery_token: crate::gallery::access::GalleryToken,
    Query(params): Query<ServePhotoQuery>,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
) -> Result<Response, AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let (mut file_path, mut mime_type, mut size_bytes, mut enc_blob_id): (
        String,
        String,
        i64,
        String,
    ) = sqlx::query_as(
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
    //
    // Runs BEFORE the rendition swap below and is keyed on the photo, which is
    // the whole authorisation story for a rung: a rendition has no access rules
    // of its own, it inherits its parent's. (The blob route cannot do this — it
    // is handed a bare blob id — which is why `is_secure_item` grew an arm that
    // resolves a rendition blob back to its photo.)
    //
    // The verdict is kept, not discarded: it also decides `Cache-Control` below.
    // A secured photo must not be written to a browser's on-disk cache, where it
    // would outlive both the unlock token and the session.
    let conf = crate::gallery::access::require_secure_access(
        &state,
        &auth.user_id,
        &photo_id,
        &gallery_token,
    )
    .await?;

    // ── Rendition selection (#49) ────────────────────────────────────────
    // Swap in the rung's locator and let every branch below run unchanged, so
    // renditions inherit Range support, chunked streaming and conditional
    // requests rather than reimplementing them.
    //
    // An unknown or unproduced rung is a 404, never a silent fallback to the
    // original: a client that asked for 1080p and was handed 4K would "work"
    // while defeating the entire point of asking.
    let etag_id = match params.rendition {
        None => photo_id.clone(),
        Some(short_edge) => {
            // Through `list_renditions`, not a bespoke query: it owns the rule
            // that an unproduced rung is not offerable, and a second copy of
            // that rule here is how a picker ends up able to request a quality
            // the picker itself was never shown. At most three rows per photo.
            let rung = crate::transcode::renditions::list_renditions(&state.read_pool, &photo_id)
                .await?
                .into_iter()
                .find(|r| r.short_edge == short_edge);

            let Some(rung) = rung else {
                tracing::warn!(
                    user_id = %auth.user_id,
                    photo_id = %photo_id,
                    short_edge,
                    "serve_photo: requested video rendition does not exist or is not yet produced"
                );
                return Err(AppError::NotFound);
            };

            // Destructured exhaustively, and deliberately not with `..`: adding
            // a field to `ServeTarget` then fails to compile until this handler
            // applies it. `content_type` was missed here on the first pass, and
            // the symptom — a `.mov`'s downscale served as `video/quicktime` —
            // is invisible server-side and only shows up as a player that will
            // not start. A unit test on the pure function cannot catch that; the
            // type system can.
            let ServeTarget {
                file_path: rung_path,
                enc_blob_id: rung_blob,
                size_bytes: rung_size,
                etag_id,
                content_type,
            } = rendition_serve_target(&photo_id, &rung);

            file_path = rung_path;
            enc_blob_id = rung_blob;
            size_bytes = rung_size;
            mime_type = content_type;
            etag_id
        }
    };

    // ── Encrypted blob serving ───────────────────────────────────────────
    // All encrypted-mode media (every photo AND video) has an empty file_path
    // and lives in an encrypted blob. Videos here are routinely multi-GB, so we
    // must NOT decrypt the whole blob into RAM per request (that caused the long
    // black screen / unresponsive download in issue #10, and is a server-memory
    // risk under concurrency). Instead: honor HTTP Range by decrypting only the
    // requested frames, and stream full responses frame-by-frame.
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
        let blob_abs = storage_root.join(&blob_storage_path);
        let content_type = HeaderValue::from_str(&mime_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream"));

        // v2 chunked blobs support seeking + frame-by-frame streaming; v1
        // monolithic blobs are below the 32 MiB chunk threshold (small).
        if peek_is_chunked(&blob_abs).await {
            // photos.size_bytes is 0 for encrypted media, so recover the true
            // plaintext length from the blob frames (no decryption needed).
            let path_for_len = blob_abs.clone();
            let total_size = tokio::task::spawn_blocking(move || {
                crate::blobs::chunked::plaintext_len_from_file(&path_for_len)
            })
            .await
            .map_err(|e| AppError::Internal(format!("Length probe panicked: {e}")))?
            .map_err(|e| AppError::Internal(format!("Length probe failed: {e}")))?;

            let etag = format!("\"{etag_id}-enc-{total_size}\"");
            if let Some(not_modified) = check_etag(&headers, &etag, conf) {
                return Ok(not_modified);
            }

            // Range request (video seek / download resume): decrypt only the
            // overlapping chunk frames → 206 Partial Content.
            if let Some(range_header) = headers.get("range").and_then(|v| v.to_str().ok()) {
                if let Some((start, end)) =
                    crate::http_utils::parse_range_header(range_header, total_size)
                {
                    let path2 = blob_abs.clone();
                    let bytes = tokio::task::spawn_blocking(move || {
                        crate::blobs::chunked::decrypt_chunked_range_from_file(
                            &key, &path2, start, end,
                        )
                    })
                    .await
                    .map_err(|e| AppError::Internal(format!("Range decrypt panicked: {e}")))?
                    .map_err(|e| AppError::Internal(format!("Range decrypt failed: {e}")))?;

                    return crate::http_utils::partial_content_builder(
                        content_type,
                        start,
                        end,
                        total_size,
                    )?
                    .header(
                        "ETag",
                        HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")),
                    )
                    .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D))
                    .body(Body::from(bytes))
                    .map_err(|e| AppError::Internal(e.to_string()));
                } else {
                    return crate::http_utils::range_not_satisfiable(total_size);
                }
            }

            // No Range: stream the whole file frame-by-frame → 200 OK.
            let body = chunked_decrypt_body(key, blob_abs);
            return Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", content_type)
                .header("Content-Length", HeaderValue::from(total_size))
                .header("Accept-Ranges", HeaderValue::from_static("bytes"))
                .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D))
                .header(
                    "ETag",
                    HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")),
                )
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()));
        }

        // v1 monolithic (small): decrypt whole, slice for Range.
        let enc_data = storage::read_blob(&storage_root, &blob_storage_path).await?;
        let raw_bytes = tokio::task::spawn_blocking(move || {
            crate::blobs::chunked::decrypt_photo_blob(&key, &enc_data)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Decrypt panicked: {e}")))?
        .map_err(|e| AppError::Internal(format!("Decrypt failed: {e}")))?;
        let total_size = raw_bytes.len() as u64;
        let etag = format!("\"{etag_id}-enc-{total_size}\"");
        if let Some(not_modified) = check_etag(&headers, &etag, conf) {
            return Ok(not_modified);
        }
        if let Some(range_header) = headers.get("range").and_then(|v| v.to_str().ok()) {
            if let Some((start, end)) =
                crate::http_utils::parse_range_header(range_header, total_size)
            {
                let slice = raw_bytes[start as usize..=end as usize].to_vec();
                return crate::http_utils::partial_content_builder(
                    content_type,
                    start,
                    end,
                    total_size,
                )?
                .header(
                    "ETag",
                    HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")),
                )
                // This 206 was the **only** exit in this handler that set no
                // `Cache-Control` at all. It was invisible while the middleware
                // stamped `no-store` over all of them; the moment media caching
                // is real, a cacheable 200 beside an uncacheable 206 means
                // seeking a small encrypted video re-decrypts it every time.
                .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D))
                .body(Body::from(slice))
                .map_err(|e| AppError::Internal(e.to_string()));
            }
        }
        return Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", content_type)
            .header("Content-Length", HeaderValue::from(total_size))
            .header("Accept-Ranges", HeaderValue::from_static("bytes"))
            .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D))
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
    let etag = format!("\"{etag_id}-{total_size}\"");
    if let Some(not_modified) = check_etag(&headers, &etag, conf) {
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

            return crate::http_utils::partial_content_builder(
                content_type,
                start,
                end,
                total_size,
            )?
            .header(
                "ETag",
                HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("")),
            )
            .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D))
            .body(body)
            .map_err(|e| AppError::Internal(e.to_string()));
        } else {
            return crate::http_utils::range_not_satisfiable(total_size);
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
        .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D))
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
    let conf = crate::gallery::access::require_secure_access(
        &state,
        &auth.user_id,
        &photo_id,
        &gallery_token,
    )
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
        if let Some(not_modified) = check_etag(&headers, &etag, conf) {
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
            .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D))
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
    if let Some(not_modified) = check_etag(&headers, &etag, conf) {
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
        .header("Cache-Control", media_cache_control(conf, MEDIA_CACHE_1D))
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
    let conf = crate::gallery::access::require_secure_access(
        &state,
        &auth.user_id,
        &photo_id,
        &gallery_token,
    )
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
        conf,
    )
    .await
}

/// GET /api/photos/:id/source-file
/// Serve the **original unconverted** source file for a converted photo.
/// Returns 404 if the photo was not converted or the source file is missing.
///
/// # This route was missing the secure-album gate entirely
///
/// Every other media projection of a photo — `file`, `web`, `thumb`,
/// `motion-video` — took a [`GalleryToken`] and called `require_secure_access`.
/// This one took neither, so an account session alone was enough to download the
/// **original, unconverted, plaintext** source of a photo sitting in a secure
/// album. Securing a photo hides its `photos` row from the gallery
/// (`ELIGIBLE_PREDICATE`) but never deletes it, and never clears `source_path`,
/// so the row this handler reads survives securing untouched.
///
/// The exposure is worse than the endpoint's name suggests: the *source* file is
/// the pre-conversion original (the HEIC, the `.mkv`), which is exactly the copy
/// the rest of the pipeline works to encrypt or replace.
///
/// [`GalleryToken`]: crate::gallery::access::GalleryToken
pub async fn serve_source_file(
    State(state): State<AppState>,
    auth: AuthUser,
    gallery_token: crate::gallery::access::GalleryToken,
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

    // Secure-album gate (see `serve_photo`). Placed after the ownership-scoped
    // lookup so a genuine 404 still wins and this cannot be used as an
    // existence oracle — the same ordering every other media handler uses.
    let conf = crate::gallery::access::require_secure_access(
        &state,
        &auth.user_id,
        &photo_id,
        &gallery_token,
    )
    .await?;

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

    let mut resp = serve_file_with_range(
        &full_path,
        total_size,
        content_type,
        &headers,
        Some(&etag),
        conf,
    )
    .await?;

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

#[cfg(test)]
mod rendition_tests {
    use super::*;
    use crate::transcode::renditions::StoredRendition;

    fn rung(short_edge: i64, blob: Option<&str>, path: Option<&str>) -> StoredRendition {
        StoredRendition {
            photo_id: "p1".into(),
            short_edge,
            width: 1920,
            height: short_edge,
            is_source: 0,
            blob_id: blob.map(str::to_string),
            file_path: path.map(str::to_string),
            codec: Some("h264".into()),
            bitrate: None,
            size_bytes: 2048,
        }
    }

    /// Encrypted mode: the rung's blob must route into the encrypted-blob
    /// branch, which this handler selects on `file_path` being EMPTY. Leaving
    /// the photo's own path in place would stream the 4K original in response
    /// to a 1080p request.
    #[test]
    fn a_blob_rung_routes_to_the_encrypted_branch() {
        let t = rendition_serve_target("p1", &rung(1080, Some("rb1"), None));
        assert_eq!(
            t.file_path, "",
            "empty file_path is what selects the blob branch"
        );
        assert_eq!(t.enc_blob_id, "rb1");
        assert_eq!(t.size_bytes, 2048);
    }

    /// Unencrypted install: the rung is a plaintext file, and the photo's
    /// encrypted blob id — if it somehow had one — must not survive the swap.
    #[test]
    fn a_file_rung_routes_to_the_plaintext_branch() {
        let t = rendition_serve_target("p1", &rung(1080, None, Some("renditions/u1/p1.1080.mp4")));
        assert_eq!(t.file_path, "renditions/u1/p1.1080.mp4");
        assert_eq!(t.enc_blob_id, "");
    }

    /// **The cache-poisoning guard.** Every rung, and the original, must have a
    /// distinct cache identity. The ETag is completed downstream with the byte
    /// length, so two rungs that happened to encode to the same size would
    /// otherwise collide and a client would be served its cached copy of the
    /// wrong quality — the one failure here that looks like the picker working.
    #[test]
    fn every_rung_has_a_distinct_cache_identity() {
        let source = rendition_serve_target("p1", &rung(2160, Some("b0"), None));
        let downscale = rendition_serve_target("p1", &rung(1080, Some("b1"), None));

        assert_ne!(source.etag_id, downscale.etag_id);
        // ...and neither may collide with the un-suffixed original's.
        assert_ne!(source.etag_id, "p1");
        assert_ne!(downscale.etag_id, "p1");
        // Distinct across photos too, since the id is the prefix.
        assert_ne!(
            rendition_serve_target("p2", &rung(1080, Some("b1"), None)).etag_id,
            downscale.etag_id
        );
    }

    /// A row with no locator cannot be served. `list_renditions` filters these
    /// out before the handler ever sees one, so this pins the belt-and-braces:
    /// the result must be un-servable, never a quiet fallback to the original.
    #[test]
    fn a_rung_with_no_locator_yields_nothing_to_serve() {
        let t = rendition_serve_target("p1", &rung(1080, None, None));
        assert_eq!(t.file_path, "");
        assert_eq!(t.enc_blob_id, "");
    }

    /// **The type must describe the rung, not its source.** The handler
    /// otherwise reuses the parent photo's `mime_type`, and the ladder always
    /// emits H.264 in MP4 — so every `.mov` in the library (10 live) would have
    /// its downscale served as `video/quicktime`, promising the player a
    /// container it is not being given.
    #[test]
    fn a_rung_is_typed_as_mp4_regardless_of_its_source_container() {
        let t = rendition_serve_target("p1", &rung(1080, Some("rb1"), None));
        assert_eq!(t.content_type, "video/mp4");
        assert_eq!(
            rendition_serve_target("p1", &rung(1080, None, Some("r/p1.1080.mp4"))).content_type,
            "video/mp4",
            "storage mode must not change the type"
        );
    }
}
