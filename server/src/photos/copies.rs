//! Photo duplication and edit copy management endpoints.
//!
//! **Duplicate photo** (`POST /api/photos/:id/duplicate`):
//! Creates a fully independent rendered copy of the photo.  When
//! `crop_metadata` is provided, the server uses **ffmpeg** (video/audio)
//! or the **image crate** (photos) to bake the edits into a new file
//! with its own `file_path`, `thumb_path`, correct `width`/`height`,
//! and `crop_metadata = NULL`.
//!
//! When no crop_metadata is given the original file is copied verbatim
//! so the duplicate is still a fully independent file on disk.
//!
//! **Edit copies** (`POST/GET/DELETE /api/photos/:id/copies`):
//! Lightweight metadata-only "versions" stored as JSON in the `edit_copies`
//! table. Each copy records crop parameters, filters, etc. without
//! duplicating the file or photos row.

use std::path::Path as StdPath;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use tokio::process::Command;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::process::{run_with_timeout, FFMPEG_RENDER_TIMEOUT};
use crate::sanitize;
use crate::state::AppState;

use super::metadata::extract_media_metadata_async;
use super::models::Photo;
use super::thumbnail::generate_thumbnail_file;

// ── Duplicate Photo (Save as Copy) ─────────────────────────────────────────

/// Request body for `POST /api/photos/{id}/duplicate`.
/// When `crop_metadata` is provided the edits are baked into a new rendered
/// file; the copy's `crop_metadata` will be `NULL`.
#[derive(Debug, Deserialize)]
pub struct DuplicatePhotoRequest {
    pub crop_metadata: Option<String>,
}

/// Parsed edit parameters — mirrors `render.rs::CropMeta`.
#[derive(Debug, Deserialize)]
struct CropMeta {
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    rotate: Option<f64>,
    brightness: Option<f64>,
    #[serde(rename = "trimStart")]
    trim_start: Option<f64>,
    #[serde(rename = "trimEnd")]
    trim_end: Option<f64>,
}

/// POST /api/photos/:id/duplicate — render a fully independent copy.
///
/// When `crop_metadata` is supplied, edits are applied via ffmpeg (video/audio)
/// or the image crate (images) and baked into a new file.  The resulting
/// `photos` row has its own `file_path`, `thumb_path`, correct dimensions,
/// and `crop_metadata = NULL`.
///
/// When no crop_metadata is given, the original file is copied verbatim.
pub async fn duplicate_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(req): Json<DuplicatePhotoRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // ── Fetch original ───────────────────────────────────────────────────
    let original: Option<Photo> = sqlx::query_as(
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

    let original = original.ok_or(AppError::NotFound)?;

    // ── Validate crop_metadata JSON if provided ──────────────────────────
    let meta_json: Option<String> = req
        .crop_metadata
        .as_deref()
        .map(|m| {
            let sanitized = sanitize::sanitize_freeform(m, 2048);
            if serde_json::from_str::<serde_json::Value>(&sanitized).is_err() {
                return Err(AppError::BadRequest(
                    "crop_metadata must be valid JSON".into(),
                ));
            }
            Ok(sanitized)
        })
        .transpose()?;

    let meta: Option<CropMeta> = meta_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let new_id = Uuid::new_v4().to_string();

    // ── Prepare output path ──────────────────────────────────────────────
    let storage_root = (**state.storage_root.load()).clone();
    let uploads_dir = storage_root.join("uploads");
    tokio::fs::create_dir_all(&uploads_dir).await.map_err(|e| {
        AppError::Internal(format!("Failed to create uploads directory: {e}"))
    })?;

    let ext = StdPath::new(&original.filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let copy_disk_name = format!("copy-{}.{}", new_id, ext);
    let copy_abs = uploads_dir.join(&copy_disk_name);
    let copy_rel = format!("uploads/{}", copy_disk_name);

    // ── Build "Copy of <filename>" display name ──────────────────────────
    let copy_filename = if original.filename.starts_with("Copy of ") {
        original.filename.clone()
    } else {
        format!("Copy of {}", original.filename)
    };

    let source_abs = storage_root.join(&original.file_path);
    if !tokio::fs::try_exists(&source_abs).await.unwrap_or(false) {
        return Err(AppError::NotFound);
    }

    let media_type = original.media_type.as_str();
    let has_edits = meta.is_some();

    // ── Render or copy the file ──────────────────────────────────────────
    if has_edits && (media_type == "video" || media_type == "audio") {
        // ─── Video/audio: use ffmpeg ─────────────────────────────────────
        render_video_copy(&source_abs, &copy_abs, media_type, meta.as_ref().unwrap(), ext).await?;
    } else if has_edits && media_type == "photo" {
        // ─── Image: use image crate ──────────────────────────────────────
        render_image_copy(&source_abs, &copy_abs, meta.as_ref().unwrap()).await?;
    } else {
        // ─── No edits: plain file copy ───────────────────────────────────
        tokio::fs::copy(&source_abs, &copy_abs).await.map_err(|e| {
            AppError::Internal(format!("Failed to copy file: {e}"))
        })?;
    }

    // ── Probe rendered file for dimensions and size ──────────────────────
    let file_meta = tokio::fs::metadata(&copy_abs).await.map_err(|e| {
        AppError::Internal(format!("Failed to stat rendered copy: {e}"))
    })?;
    let size_bytes = file_meta.len() as i64;

    let (new_w, new_h, _, _, _, _) =
        extract_media_metadata_async(copy_abs.clone()).await;

    // For video copies, probe the new duration
    let new_duration = if media_type == "video" || media_type == "audio" {
        super::thumbnail::probe_duration(&copy_abs).await
    } else {
        None
    };

    // ── Generate thumbnail from the rendered file ────────────────────────
    let thumb_ext = if original.mime_type == "image/gif" { "gif" } else { "jpg" };
    let thumb_rel = format!(".thumbnails/{}.thumb.{}", new_id, thumb_ext);
    let thumb_abs = storage_root.join(&thumb_rel);
    let thumb_rel_opt = {
        let mime_clone = original.mime_type.clone();
        let copy_abs_c = copy_abs.clone();
        let thumb_abs_c = thumb_abs.clone();
        let ok = generate_thumbnail_file(&copy_abs_c, &thumb_abs_c, &mime_clone, None).await;
        if ok { Some(thumb_rel.clone()) } else { None }
    };

    // ── Use the original's taken_at (for timeline ordering) ──────────────
    let created_at = original.created_at.clone();

    // ── Insert DB row — crop_metadata is NULL (edits are baked in) ───────
    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
         size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
         thumb_path, created_at, encrypted_blob_id, encrypted_thumb_blob_id, \
         is_favorite, crop_metadata, camera_model, photo_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '', '', 0, NULL, ?, NULL)",
    )
    .bind(&new_id)
    .bind(&auth.user_id)
    .bind(&copy_filename)
    .bind(&copy_rel)
    .bind(&original.mime_type)
    .bind(&original.media_type)
    .bind(size_bytes)
    .bind(new_w)
    .bind(new_h)
    .bind(new_duration.or(original.duration_secs))
    .bind(&original.taken_at)
    .bind(original.latitude)
    .bind(original.longitude)
    .bind(&thumb_rel_opt)
    .bind(&created_at)
    .bind(&original.camera_model)
    .execute(&state.pool)
    .await?;

    tracing::info!(
        "Rendered duplicate {} → {} ({}×{}) for user {}",
        photo_id, new_id, new_w, new_h, auth.user_id
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": new_id,
            "source_photo_id": photo_id,
            "filename": copy_filename,
            "crop_metadata": null,
            "width": new_w,
            "height": new_h,
            "duration_secs": new_duration.or(original.duration_secs),
            "mime_type": original.mime_type,
            "media_type": original.media_type,
            "size_bytes": size_bytes,
        })),
    ))
}

// ── ffmpeg video/audio rendering ─────────────────────────────────────────────

/// Render a video/audio file with crop, rotation, brightness, and trim.
/// Mirrors the logic in `render.rs` but writes to a permanent file instead of
/// a temp cache entry.
async fn render_video_copy(
    source: &std::path::Path,
    dest: &std::path::Path,
    media_type: &str,
    meta: &CropMeta,
    ext: &str,
) -> Result<(), AppError> {
    let (trim_start, trim_end) = (
        meta.trim_start.unwrap_or(0.0),
        meta.trim_end.unwrap_or(0.0),
    );
    let apply_trim_start = trim_start > 0.01;
    let apply_trim_end = trim_end > 0.01 && trim_end > trim_start + 0.01;

    let needs_video_filter = media_type == "video" && {
        let has_crop = meta.width.unwrap_or(1.0) < 0.999
            || meta.height.unwrap_or(1.0) < 0.999
            || meta.x.unwrap_or(0.0) > 0.001
            || meta.y.unwrap_or(0.0) > 0.001;
        let has_rotate = meta.rotate.unwrap_or(0.0).abs() > 0.5;
        let has_brightness = meta.brightness.unwrap_or(0.0).abs() > 0.5;
        has_crop || has_rotate || has_brightness
    };

    let mut args: Vec<String> = vec![
        "-y".into(),
        "-i".into(),
        source.to_string_lossy().into_owned(),
    ];

    if apply_trim_start {
        args.push("-ss".into());
        args.push(format!("{:.6}", trim_start));
    }
    if apply_trim_end {
        args.push("-to".into());
        args.push(format!("{:.6}", trim_end));
    }

    if needs_video_filter {
        let mut filters: Vec<String> = Vec::new();

        // Crop
        let x = meta.x.unwrap_or(0.0);
        let y = meta.y.unwrap_or(0.0);
        let w = meta.width.unwrap_or(1.0);
        let h = meta.height.unwrap_or(1.0);
        if w < 0.999 || h < 0.999 || x > 0.001 || y > 0.001 {
            filters.push(format!(
                "crop=iw*{w:.6}:ih*{h:.6}:iw*{x:.6}:ih*{y:.6}"
            ));
        }

        // Rotation
        let rot = ((meta.rotate.unwrap_or(0.0) as i32).rem_euclid(360)) as u32;
        match rot {
            90 => filters.push("transpose=1".into()),
            180 => {
                filters.push("vflip".into());
                filters.push("hflip".into());
            }
            270 => filters.push("transpose=2".into()),
            _ => {}
        }

        // Brightness
        let b = meta.brightness.unwrap_or(0.0);
        if b.abs() > 0.5 {
            filters.push(format!("eq=brightness={:.4}", b / 100.0));
        }

        if !filters.is_empty() {
            args.push("-vf".into());
            args.push(filters.join(","));
        }

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
    } else if apply_trim_start || apply_trim_end {
        // Trim-only: copy streams losslessly
        args.extend(["-c".into(), "copy".into()]);
    } else {
        // No meaningful edits — shouldn't normally reach here, but be safe
        args.extend(["-c".into(), "copy".into()]);
    }

    if ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }

    args.push(dest.to_string_lossy().into_owned());

    tracing::info!("[duplicate/render] ffmpeg args: {:?}", args);

    let mut cmd = Command::new("ffmpeg");
    cmd.args(&args);
    let output = run_with_timeout(&mut cmd, FFMPEG_RENDER_TIMEOUT)
        .await
        .map_err(|e| AppError::Internal(format!("ffmpeg render for copy: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("[duplicate/render] ffmpeg failed:\n{}", stderr);
        let last_line = stderr.lines().last().unwrap_or("unknown error").to_string();
        return Err(AppError::Internal(format!(
            "ffmpeg render for copy failed: {last_line}"
        )));
    }

    Ok(())
}

// ── image crate rendering (photos) ───────────────────────────────────────────

/// Render a static image with crop, rotation, and brightness edits.
async fn render_image_copy(
    source: &std::path::Path,
    dest: &std::path::Path,
    meta: &CropMeta,
) -> Result<(), AppError> {
    let src = source.to_path_buf();
    let dst = dest.to_path_buf();
    let x = meta.x.unwrap_or(0.0);
    let y = meta.y.unwrap_or(0.0);
    let w = meta.width.unwrap_or(1.0);
    let h = meta.height.unwrap_or(1.0);
    let rot = ((meta.rotate.unwrap_or(0.0) as i32).rem_euclid(360)) as u32;
    let brightness = meta.brightness.unwrap_or(0.0);

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let mut img = image::open(&src)
            .map_err(|e| AppError::Internal(format!("Failed to open image for copy: {e}")))?;

        let iw = img.width() as f64;
        let ih = img.height() as f64;

        // Crop (fractional coordinates, clamped to image bounds)
        if w < 0.999 || h < 0.999 || x > 0.001 || y > 0.001 {
            let cx = ((x * iw).round() as u32).min(img.width().saturating_sub(1));
            let cy = ((y * ih).round() as u32).min(img.height().saturating_sub(1));
            let max_w = img.width().saturating_sub(cx);
            let max_h = img.height().saturating_sub(cy);
            let cw = ((w * iw).round().max(1.0) as u32).min(max_w).max(1);
            let ch = ((h * ih).round().max(1.0) as u32).min(max_h).max(1);
            img = img.crop_imm(cx, cy, cw, ch);
        }

        // Rotation
        img = match rot {
            90 => img.rotate90(),
            180 => img.rotate180(),
            270 => img.rotate270(),
            _ => img,
        };

        // Brightness (simple linear adjustment: pixel * (1 + brightness/100))
        if brightness.abs() > 0.5 {
            let factor = 1.0 + brightness / 100.0;
            img = image::DynamicImage::ImageRgba8(image::imageops::brighten(&img, (factor * 10.0) as i32));
        }

        // Determine output format from extension
        let ext = dst.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
        let format = match ext.to_ascii_lowercase().as_str() {
            "png" => image::ImageFormat::Png,
            "gif" => image::ImageFormat::Gif,
            "webp" => image::ImageFormat::WebP,
            "bmp" => image::ImageFormat::Bmp,
            _ => image::ImageFormat::Jpeg,
        };

        img.save_with_format(&dst, format)
            .map_err(|e| AppError::Internal(format!("Failed to save rendered image copy: {e}")))?;

        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("Image render task panicked: {e}")))?
}

// ── Edit Copies (Save Copy) ────────────────────────────────────────────────

/// Request body for `POST /api/photos/{id}/copies`.
/// Creates a metadata-only edit copy of a photo — stores the edit parameters
/// (brightness, rotation, filter, etc.) without duplicating the underlying file.
#[derive(Debug, Deserialize)]
pub struct CreateEditCopyRequest {
    pub name: Option<String>,
    pub edit_metadata: String,
}

/// POST /api/photos/:id/copies — create a metadata-only "copy" of a photo/video/audio
pub async fn create_edit_copy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(req): Json<CreateEditCopyRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify the photo belongs to this user
    let exists: bool =
        sqlx::query_scalar("SELECT COUNT(*) > 0 FROM photos WHERE id = ? AND user_id = ?")
            .bind(&photo_id)
            .bind(&auth.user_id)
            .fetch_one(&state.pool)
            .await?;

    if !exists {
        return Err(AppError::NotFound);
    }

    // Validate edit_metadata is valid JSON
    let meta = sanitize::sanitize_freeform(&req.edit_metadata, 2048);
    if serde_json::from_str::<serde_json::Value>(&meta).is_err() {
        return Err(AppError::BadRequest(
            "edit_metadata must be valid JSON".into(),
        ));
    }

    let copy_id = Uuid::new_v4().to_string();
    let name = req
        .name
        .as_deref()
        .map(|n| sanitize::sanitize_freeform(n, 128))
        .unwrap_or_else(|| {
            let now = Utc::now().format("%Y-%m-%d %H:%M").to_string();
            format!("Copy {}", now)
        });

    sqlx::query(
        "INSERT INTO edit_copies (id, photo_id, user_id, name, edit_metadata) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&copy_id)
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&name)
    .bind(&meta)
    .execute(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({
        "id": copy_id,
        "photo_id": photo_id,
        "name": name,
        "edit_metadata": serde_json::from_str::<serde_json::Value>(&meta).ok(),
    })))
}

/// GET /api/photos/:id/copies — list all edit copies for a photo
pub async fn list_edit_copies(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT id, name, edit_metadata, created_at FROM edit_copies WHERE photo_id = ? AND user_id = ? ORDER BY created_at DESC",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let copies: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, name, meta, created_at)| {
            serde_json::json!({
                "id": id,
                "name": name,
                "edit_metadata": serde_json::from_str::<serde_json::Value>(&meta).ok(),
                "created_at": created_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "copies": copies })))
}

/// DELETE /api/photos/:id/copies/:copy_id — delete a single edit copy
pub async fn delete_edit_copy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((photo_id, copy_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = sqlx::query("DELETE FROM edit_copies WHERE id = ? AND photo_id = ? AND user_id = ?")
        .bind(&copy_id)
        .bind(&photo_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}
