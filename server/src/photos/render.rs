//! ffmpeg-backed render endpoint for video and audio.
//!
//! `POST /api/photos/:id/render` accepts the same `crop_metadata` JSON the
//! client stores (crop rect, rotation, brightness, trim start/end) and
//! returns a rendered file as an attachment.  Images are handled client-side
//! via Canvas 2D and must not be sent here.
//!
//! The endpoint shells out to `ffmpeg` which must be installed on the host.
//! The Dockerfile and install scripts ensure this for all supported platforms.

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
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::Photo;

// ── Request / metadata types ──────────────────────────────────────────────────

/// Request body for `POST /api/photos/:id/render`.
#[derive(Debug, Deserialize)]
pub struct RenderRequest {
    /// JSON string of crop/edit metadata produced by the client editor.
    /// If omitted the server falls back to the `crop_metadata` stored in the
    /// `photos` row for this photo.
    pub crop_metadata: Option<String>,
}

/// Parsed edit parameters.  All fields are optional so partial metadata is
/// handled gracefully — a missing field means "use the default/neutral value".
#[derive(Debug, Deserialize)]
struct CropMeta {
    /// Left edge of crop rect, 0–1 fraction of original width.
    x: Option<f64>,
    /// Top edge of crop rect, 0–1 fraction of original height.
    y: Option<f64>,
    /// Width of crop rect, 0–1 fraction of original width.  Default 1.0.
    width: Option<f64>,
    /// Height of crop rect, 0–1 fraction of original height.  Default 1.0.
    height: Option<f64>,
    /// Clockwise rotation in degrees.  Only 0 / 90 / 180 / 270 are supported.
    rotate: Option<f64>,
    /// Brightness adjustment, -100 (darkest) to +100 (brightest).  Default 0.
    brightness: Option<f64>,
    /// Trim start in seconds.  Omit or 0 = start of file.
    #[serde(rename = "trimStart")]
    trim_start: Option<f64>,
    /// Trim end in seconds.  Omit or 0 = end of file.
    #[serde(rename = "trimEnd")]
    trim_end: Option<f64>,
}

// ── Handler ───────────────────────────────────────────────────────────────────

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
        let probe = Command::new("ffmpeg")
            .arg("-version")
            .output()
            .await
            .ok();
        if probe.map_or(true, |o| !o.status.success()) {
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
         crop_metadata, camera_model, photo_hash \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let photo = photo.ok_or(AppError::NotFound)?;

    // ── Only video / audio are rendered server-side ───────────────────────────
    let media_type = photo.media_type.as_str();
    if media_type != "video" && media_type != "audio" {
        return Err(AppError::BadRequest(
            "render is only for video and audio; apply image edits client-side via Canvas".into(),
        ));
    }

    // ── Resolve source file ───────────────────────────────────────────────────
    let source_path = state.config.storage.root.join(&photo.file_path);
    if !tokio::fs::try_exists(&source_path).await.unwrap_or(false) {
        tracing::error!(
            "[render] source file not found: {}",
            source_path.display()
        );
        return Err(AppError::NotFound);
    }

    // ── Parse edit metadata (request body takes priority over DB row) ─────────
    let meta_str = req
        .crop_metadata
        .or_else(|| photo.crop_metadata.clone());

    let meta: Option<CropMeta> = meta_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    // ── Classify what edits are requested ────────────────────────────────────
    let (trim_start, trim_end) = meta.as_ref().map_or((0.0_f64, 0.0_f64), |m| {
        (m.trim_start.unwrap_or(0.0), m.trim_end.unwrap_or(0.0))
    });
    let apply_trim_start = trim_start > 0.01;
    let apply_trim_end = trim_end > 0.01 && trim_end > trim_start + 0.01;

    let needs_video_filter = media_type == "video"
        && meta.as_ref().map_or(false, |m| {
            let has_crop = m.width.unwrap_or(1.0) < 0.999
                || m.height.unwrap_or(1.0) < 0.999
                || m.x.unwrap_or(0.0) > 0.001
                || m.y.unwrap_or(0.0) > 0.001;
            let has_rotate = m.rotate.unwrap_or(0.0).abs() > 0.5;
            let has_brightness = m.brightness.unwrap_or(0.0).abs() > 0.5;
            has_crop || has_rotate || has_brightness
        });

    // ── Derive output extension from original filename ────────────────────────
    let ext = Path::new(&photo.filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4");

    // ── Check render cache ────────────────────────────────────────────────────
    // Cache key = SHA-256 of the resolved metadata string (or literal "default"
    // when no edits are stored).  First 16 hex chars is enough for collision
    // resistance within a single user's library.
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
        let data = tokio::fs::read(&cache_path)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read render cache: {e}")))?;
        return Ok(build_response(data, &photo.mime_type, &photo.filename).into_response());
    }

    let tmp_path = std::env::temp_dir().join(format!("sp-render-{}.{}", Uuid::new_v4(), ext));

    // ── Build ffmpeg argument list ────────────────────────────────────────────
    // We use output-side seeking (ss/to placed after -i) for frame accuracy.
    // This is slightly slower than input seeking but correct for all cases.
    let mut args: Vec<String> = vec!["-y".into(), "-i".into(), source_path.to_string_lossy().into_owned()];

    if apply_trim_start {
        args.push("-ss".into());
        args.push(format!("{:.6}", trim_start));
    }
    if apply_trim_end {
        args.push("-to".into());
        args.push(format!("{:.6}", trim_end));
    }

    if needs_video_filter {
        // Build comma-separated filter chain
        let mut filters: Vec<String> = Vec::new();

        if let Some(ref m) = meta {
            // Crop filter (expressed as fractions of input dimensions)
            let x = m.x.unwrap_or(0.0);
            let y = m.y.unwrap_or(0.0);
            let w = m.width.unwrap_or(1.0);
            let h = m.height.unwrap_or(1.0);
            if w < 0.999 || h < 0.999 || x > 0.001 || y > 0.001 {
                // ffmpeg crop filter: crop=out_w:out_h:x:y (all in pixels)
                // Using expressions so ffmpeg evaluates at runtime with real dims
                filters.push(format!(
                    "crop=iw*{w:.6}:ih*{h:.6}:iw*{x:.6}:ih*{y:.6}",
                ));
            }

            // Rotation via transpose filter (only cardinal angles supported)
            let rot = ((m.rotate.unwrap_or(0.0) as i32).rem_euclid(360)) as u32;
            match rot {
                90 => filters.push("transpose=1".into()),   // 90° clockwise
                180 => {
                    filters.push("vflip".into());
                    filters.push("hflip".into());
                }
                270 => filters.push("transpose=2".into()),  // 90° counter-clockwise
                _ => {}
            }

            // Brightness via eq filter (-1.0 to 1.0; our scale is -100 to +100)
            let b = m.brightness.unwrap_or(0.0);
            if b.abs() > 0.5 {
                filters.push(format!("eq=brightness={:.4}", b / 100.0));
            }
        }

        if !filters.is_empty() {
            args.push("-vf".into());
            args.push(filters.join(","));
        }

        // Re-encode: H.264 + AAC in a fast, high-quality preset
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "fast".into(),
            "-crf".into(),
            "18".into(),
            "-c:a".into(),
            "aac".into(),
        ]);
    } else {
        // Trim-only (or audio): copy all streams losslessly — fast and no quality loss
        args.extend(["-c".into(), "copy".into()]);
    }

    // For MP4: move the moov atom to the front so the browser can start playing
    // immediately without downloading the whole file first.
    if ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }

    args.push(tmp_path.to_string_lossy().into_owned());

    tracing::info!("[render] ffmpeg args: {:?}", args);

    // ── Run ffmpeg ────────────────────────────────────────────────────────────
    let output = Command::new("ffmpeg")
        .args(&args)
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("ffmpeg spawn failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("[render] ffmpeg failed:\n{}", stderr);
        let _ = tokio::fs::remove_file(&tmp_path).await;
        let last_line = stderr.lines().last().unwrap_or("unknown error").to_string();
        return Err(AppError::Internal(format!("ffmpeg render failed: {last_line}")));
    }

    // ── Read rendered file, save to cache, and stream back ───────────────────
    let data = tokio::fs::read(&tmp_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read rendered file: {e}")))?;

    // Save to cache (best-effort — don't fail the request if the write fails)
    if let Err(e) = tokio::fs::copy(&tmp_path, &cache_path).await {
        tracing::warn!("[render] failed to write render cache {:?}: {}", cache_path, e);
    } else {
        tracing::info!("[render] cached render at {:?}", cache_path);
    }

    // Clean up temp file
    let _ = tokio::fs::remove_file(&tmp_path).await;

    Ok(build_response(data, &photo.mime_type, &photo.filename).into_response())
}

/// Build the download response headers + body for a rendered media file.
fn build_response(data: Vec<u8>, mime: &str, original_filename: &str) -> impl IntoResponse {
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
    (StatusCode::OK, headers, Body::from(data))
}
