//! On-demand render endpoint — "Download Rendered" in the editing engine.
//!
//! `POST /api/photos/:id/render` applies crop / trim / rotation / brightness
//! edits to a video or audio file using ffmpeg and streams the result back for
//! download.  Images are handled client-side via Canvas 2D.
//!
//! The endpoint maintains a `.renders/` cache directory keyed by a hash of the
//! edit metadata, so repeated downloads of the same edit are instant.

use std::path::Path;

use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use hex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::process::{run_with_timeout, FFPROBE_TIMEOUT};
use crate::state::AppState;

use crate::photos::models::Photo;

use super::ffmpeg;
use super::models::CropMeta;

/// Request body for `POST /api/photos/:id/render`.
#[derive(Debug, Deserialize)]
pub struct RenderRequest {
    /// JSON string of crop/edit metadata produced by the client editor.
    /// If omitted the server falls back to the `crop_metadata` stored in the
    /// `photos` row for this photo.
    pub crop_metadata: Option<String>,
}

/// POST /api/photos/:id/render
///
/// Apply crop / trim / rotation / brightness edits to a video or audio file
/// using ffmpeg and stream the result back for download.
///
/// - Returns **400** if the photo is an image (handle those client-side).
/// - Returns **404** if the photo does not exist or is not owned by the caller.
/// - Returns **500** if ffmpeg is not installed or the encode fails.
pub async fn render_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath(photo_id): AxumPath<String>,
    Json(req): Json<RenderRequest>,
) -> Result<Response, AppError> {
    // ── Ensure ffmpeg is available (cached after first check) ───────────────
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::OnceLock;
    static FFMPEG_CHECKED: OnceLock<AtomicBool> = OnceLock::new();
    let checked = FFMPEG_CHECKED.get_or_init(|| AtomicBool::new(false));
    if !checked.load(AtomicOrdering::Relaxed) {
        let mut probe_cmd = Command::new("ffmpeg");
        probe_cmd.arg("-version");
        let probe = run_with_timeout(&mut probe_cmd, FFPROBE_TIMEOUT).await.ok();
        if probe.as_ref().is_none_or(|o| !o.status.success()) {
            return Err(AppError::Internal(
                "ffmpeg is not installed on this server; install it and restart".into(),
            ));
        }
        checked.store(true, AtomicOrdering::Relaxed);
    }

    // ── Fetch photo row, enforcing ownership ─────────────────────────────────
    let photo: Option<Photo> = sqlx::query_as(
        "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path, \
         created_at, encrypted_blob_id, encrypted_thumb_blob_id, is_favorite, \
         crop_metadata, camera_model, photo_hash, photo_subtype, burst_id, motion_video_blob_id \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let photo = photo.ok_or(AppError::NotFound)?;

    // ── Render targets: video, audio, and animated GIF ───────────────────────
    // GIFs are stored as media_type "photo" (mime image/gif). The image crate
    // decodes only their first frame, so route GIF bakes through ffmpeg too —
    // detect by mime, not media_type.
    let media_type = photo.media_type.as_str();
    let is_gif = photo.mime_type == "image/gif";
    if media_type != "video" && media_type != "audio" && !is_gif {
        return Err(AppError::BadRequest(
            "render is only for video, audio, and animated GIFs; apply other image edits \
             client-side via Canvas"
                .into(),
        ));
    }

    // ── Parse edit metadata (request body takes priority over DB row) ─────────
    let meta_str = req.crop_metadata.or_else(|| photo.crop_metadata.clone());
    let meta: Option<CropMeta> = meta_str.as_deref().and_then(CropMeta::from_json);

    tracing::info!(
        "[render] photo_id={}, media_type={}, is_gif={}, dims={}×{}, \
         has_meta={}, meta_str={:?}",
        photo_id,
        media_type,
        is_gif,
        photo.width,
        photo.height,
        meta.is_some(),
        meta_str.as_deref().unwrap_or("none"),
    );
    if let Some(ref m) = meta {
        tracing::info!(
            "[render] CropMeta: rotate={}°, has_crop={}, has_brightness={}, \
             has_trim={}, swaps_dims={}",
            m.rotation_degrees(),
            m.has_crop(),
            m.has_brightness(),
            m.has_trim(),
            m.rotation_swaps_dimensions(),
        );
    }

    // ── Derive output extension from original filename ────────────────────────
    // Restrict the extension to alphanumeric characters to prevent path-component
    // injection from the user-supplied filename stored in the database.
    let ext = Path::new(&photo.filename)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().all(|c| c.is_alphanumeric()))
        .unwrap_or(if is_gif { "gif" } else { "mp4" });

    // ── Check render cache (before touching the source) ───────────────────────
    let meta_key = meta_str.as_deref().unwrap_or("default");
    let mut hasher = Sha256::new();
    hasher.update(meta_key.as_bytes());
    let crop_hash = hex::encode(hasher.finalize());
    let crop_hash = &crop_hash[..16];

    let cache_dir = state.config.storage.root.join(".renders");
    tokio::fs::create_dir_all(&cache_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create render cache dir: {e}")))?;

    let cache_path = cache_dir.join(format!("{}-{}.{}", photo.id, crop_hash, ext));

    if tokio::fs::try_exists(&cache_path).await.unwrap_or(false) {
        tracing::info!("[render] cache hit: {:?}", cache_path);
        return stream_file_response(&cache_path, &photo.mime_type, &photo.filename).await;
    }

    // ── Resolve source (cache miss only) ──────────────────────────────────────
    // Prefer the on-disk plaintext file; blob-only media (mobile-uploaded, no
    // file_path) is decrypted to a temp file so ffmpeg can read it.
    let storage_root = (**state.storage_root.load()).clone();
    let (source_path, temp_source): (std::path::PathBuf, Option<std::path::PathBuf>) =
        if !photo.file_path.is_empty() {
            // Validate the stored file path is a safe relative path before joining
            // to the storage root — belts-and-suspenders since the path was
            // validated at upload time.
            if let Err(reason) = crate::sanitize::validate_relative_path(&photo.file_path) {
                tracing::error!(
                    "[render] invalid stored file path '{}': {}",
                    photo.file_path,
                    reason
                );
                return Err(AppError::NotFound);
            }
            // Canonicalize the storage root so the containment check is resilient
            // to symlinks and `..` traversal in `photo.file_path`.
            let canonical_root = tokio::fs::canonicalize(&state.config.storage.root)
                .await
                .map_err(|e| {
                    tracing::error!("[render] cannot canonicalize storage root: {e}");
                    AppError::Internal("storage root unavailable".into())
                })?;
            let candidate = canonical_root.join(&photo.file_path);
            let resolved = match tokio::fs::canonicalize(&candidate).await {
                Ok(p) if p.starts_with(&canonical_root) => p,
                Ok(p) => {
                    tracing::error!("[render] resolved path '{}' escapes storage root", p.display());
                    return Err(AppError::NotFound);
                }
                Err(_) => {
                    tracing::error!("[render] source file not found: {}", candidate.display());
                    return Err(AppError::NotFound);
                }
            };
            (resolved, None)
        } else if !photo.encrypted_blob_id.is_empty() {
            let tmp = decrypt_blob_to_temp(&state, &auth.user_id, &photo, &storage_root, ext).await?;
            (tmp.clone(), Some(tmp))
        } else {
            tracing::error!("[render] photo has neither file_path nor encrypted_blob_id");
            return Err(AppError::NotFound);
        };

    // ── Render via shared ffmpeg module ───────────────────────────────────────
    let tmp_dir = state.config.storage.root.join(".tmp");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Create tmp dir: {e}")))?;
    let tmp_path = tmp_dir.join(format!("sp-render-{}.{}", Uuid::new_v4(), ext));

    // Build a default CropMeta if none was provided (no-op edits → stream copy)
    let default_meta = CropMeta {
        x: None,
        y: None,
        width: None,
        height: None,
        rotate: None,
        brightness: None,
        trim_start: None,
        trim_end: None,
    };
    let effective_meta = meta.as_ref().unwrap_or(&default_meta);

    let render_result = if is_gif {
        ffmpeg::run_gif_render(&source_path, &tmp_path, effective_meta).await
    } else {
        ffmpeg::run_ffmpeg_render(&source_path, &tmp_path, media_type, effective_meta, ext).await
    };

    // Always remove a decrypted temp source, whether the render succeeded or not.
    if let Some(ref ts) = temp_source {
        let _ = tokio::fs::remove_file(ts).await;
    }

    render_result.inspect_err(|_e| {
        // Clean up tmp output on failure
        let tp = tmp_path.clone();
        tokio::spawn(async move {
            let _ = tokio::fs::remove_file(&tp).await;
        });
    })?;

    // ── Save to cache and stream back ─────────────────────────────────────────
    if let Err(e) = tokio::fs::rename(&tmp_path, &cache_path).await {
        tracing::warn!("[render] failed to cache render {:?}: {}", cache_path, e);
        let resp = stream_file_response(&tmp_path, &photo.mime_type, &photo.filename).await;
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return resp;
    }
    tracing::info!("[render] cached render at {:?}", cache_path);

    stream_file_response(&cache_path, &photo.mime_type, &photo.filename).await
}

/// Decrypt a photo's encrypted blob to a temp file so ffmpeg can read it.
///
/// Used for blob-only media (mobile-uploaded, no plaintext `file_path`). The
/// caller is responsible for deleting the returned temp file. The filename
/// embeds a fresh UUID to avoid predictable-name attacks, and `ext` is the
/// already-sanitized output extension.
async fn decrypt_blob_to_temp(
    state: &AppState,
    user_id: &str,
    photo: &Photo,
    storage_root: &std::path::Path,
    ext: &str,
) -> Result<std::path::PathBuf, AppError> {
    let blob_id = &photo.encrypted_blob_id;
    let enc_key = crate::crypto::load_wrapped_key(&state.pool, &state.config.auth.jwt_secret)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to load encryption key: {e}")))?
        .ok_or_else(|| AppError::Internal("No encryption key configured".into()))?;

    let blob_row: Option<(String,)> =
        sqlx::query_as("SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?")
            .bind(blob_id)
            .bind(user_id)
            .fetch_optional(&state.read_pool)
            .await?;
    let (blob_storage_path,) = blob_row.ok_or(AppError::NotFound)?;

    let enc_data = crate::blobs::storage::read_blob(storage_root, &blob_storage_path).await?;
    // Format-aware: handles the legacy monolithic envelope and the v2 chunked
    // container — see blobs/chunked.rs.
    let raw_bytes = {
        let k = enc_key;
        tokio::task::spawn_blocking(move || crate::blobs::chunked::decrypt_photo_blob(&k, &enc_data))
            .await
            .map_err(|e| AppError::Internal(format!("Decrypt panicked: {e}")))?
            .map_err(|e| AppError::Internal(format!("Decrypt failed: {e}")))?
    };

    // nosemgrep: rust.lang.security.temp-dir.temp-dir
    let tmp_path = std::env::temp_dir().join(format!("sp-render-src-{}.{}", Uuid::new_v4(), ext));
    tokio::fs::write(&tmp_path, &raw_bytes)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write temp source: {e}")))?;
    tracing::info!(
        "[render] decrypted blob {} to temp source {} ({} bytes)",
        blob_id,
        tmp_path.display(),
        raw_bytes.len(),
    );
    Ok(tmp_path)
}

/// Stream a file from disk as the download response.
///
/// Uses `ReaderStream` so we never load the entire file into memory —
/// critical for large video renders.
async fn stream_file_response(
    path: &std::path::Path,
    mime: &str,
    original_filename: &str,
) -> Result<Response, AppError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to open rendered file: {e}")))?;

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let download_filename = format!("Edited {original_filename}");
    let mut headers = HeaderMap::new();
    headers.insert(
        "Content-Type",
        HeaderValue::from_str(mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        "Content-Disposition",
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            download_filename.replace('"', "'")
        ))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    Ok((StatusCode::OK, headers, body).into_response())
}
