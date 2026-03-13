//! Filesystem scanning, thumbnail generation, and web preview creation.
//!
//! `POST /api/admin/photos/scan` walks the storage directory tree, registers
//! every unregistered media file as a plain photo, extracts EXIF metadata,
//! generates JPEG thumbnails (via ImageMagick → FFmpeg fallback), and creates
//! browser-compatible web previews for non-native formats (HEIC → JPEG,
//! MKV → MP4, etc.). Video transcoding is deferred to the background
//! conversion task in [`super::convert`].

use std::path::Path;

use axum::extract::State;
use axum::Json;
use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::state::AppState;

use super::metadata::extract_media_metadata;

/// Streaming SHA-256 hash — reads in 64 KB chunks to avoid loading entire file into memory.
/// Returns the first 12 hex chars of the hash.
async fn compute_photo_hash_streaming(path: &Path) -> Option<String> {
    let mut file = tokio::fs::File::open(path).await.ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 65536]; // 64 KB chunks
    loop {
        let n = file.read(&mut buf).await.ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hex::encode(&hasher.finalize()[..6]))
}

/// Check if FFmpeg is available on the system (public for background convert task).
pub async fn ffmpeg_available_pub() -> bool {
    ffmpeg_available().await
}

/// Check if FFmpeg is available on the system.
async fn ffmpeg_available() -> bool {
    tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Generate a JPEG thumbnail using FFmpeg.
/// For videos: extracts a frame at ~1 second.
/// For images: converts to JPEG and resizes.
/// For audio: generates a black placeholder.
pub async fn generate_thumbnail_file(
    input_path: &Path,
    output_path: &Path,
    mime: &str,
    crop_metadata: Option<&str>,
) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    // For audio, create a simple black 256×256 JPEG using the image crate (no FFmpeg needed)
    if mime.starts_with("audio/") {
        let img = image::RgbImage::from_pixel(256, 256, image::Rgb([0u8, 0, 0]));
        let mut buf = std::io::Cursor::new(Vec::new());
        if image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .is_ok()
        {
            return tokio::fs::write(output_path, buf.into_inner())
                .await
                .is_ok();
        }
        return false;
    }

    let input_str = input_path.to_str().unwrap_or("");
    let output_str = output_path.to_str().unwrap_or("");

    // ── Non-video images: try ImageMagick first ─────────────────────────
    // ImageMagick handles SVG, CR2 RAW, HEIC, and other exotic formats
    // much better than FFmpeg (which often produces near-black output for
    // vector/RAW formats).
    if !mime.starts_with("video/") {
        let magick_result = tokio::process::Command::new("convert")
            .args([
                // Read only the first frame/layer (for ICO, multi-page TIFF, etc.)
                &format!("{}[0]", input_str),
                "-thumbnail",
                "256x256>",
                "-background",
                "black",
                "-gravity",
                "center",
                "-extent",
                "256x256",
                "-quality",
                "85",
                output_str,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if matches!(magick_result, Ok(s) if s.success()) {
            return true;
        }

        // ImageMagick failed — fall through to FFmpeg for images too
    }

    // ── Video and image FFmpeg path ─────────────────────────────────────
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.arg("-y");

    // For raw video streams (.hevc, .h264, .h265), force the demuxer format
    // since they lack a container with timing info
    match mime {
        "video/hevc" => {
            cmd.args(["-f", "hevc"]);
        }
        "video/h264" => {
            cmd.args(["-f", "h264"]);
        }
        _ => {}
    }

    // For videos, seek to 1 second for a more interesting frame
    // (skip seek for raw streams — seeking is unreliable without a container)
    if mime.starts_with("video/") && mime != "video/hevc" && mime != "video/h264" {
        cmd.args(["-ss", "1"]);
    }

    let mut vf_filters = String::new();

    // If crop_metadata is provided, parse it and prepend crop/brightness filters
    // Expected JSON: { "x": 0.1, "y": 0.2, "width": 0.8, "height": 0.7, "brightness": 0, "rotate": 0 }
    if let Some(crop_str) = crop_metadata {
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(crop_str) {
            let cx = meta.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let cy = meta.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let cw = meta.get("width").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let ch = meta.get("height").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let brightness = meta.get("brightness").and_then(|v| v.as_f64()).unwrap_or(0.0);

            // Apply brightness if non-zero (ffmpeg eq=brightness expects -1.0 to 1.0)
            if brightness != 0.0 {
                // frontend sends brightness in range -100 to 100, ffmpeg uses -1.0 to 1.0
                let b = brightness / 100.0;
                vf_filters.push_str(&format!("eq=brightness={},", b));
            }
            
            // Apply crop if not default (using relative inputs iw/ih)
            if cx > 0.0 || cy > 0.0 || cw < 1.0 || ch < 1.0 {
                vf_filters.push_str(&format!("crop=iw*{}:ih*{}:iw*{}:ih*{},", cw, ch, cx, cy));
            }
        }
    }

    vf_filters.push_str("scale=256:256:force_original_aspect_ratio=decrease,pad=256:256:(ow-iw)/2:(oh-ih)/2:black");

    cmd.args(["-i", input_str])
        .args([
            "-vf",
            &vf_filters,
            "-frames:v",
            "1",
            "-q:v",
            "5",
        ])
        .arg(output_str)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    if matches!(cmd.status().await, Ok(s) if s.success()) {
        return true;
    }

    // FFmpeg also failed — last resort ImageMagick for video stills (unlikely)
    if mime.starts_with("video/") {
        let magick_result = tokio::process::Command::new("convert")
            .args([
                &format!("{}[0]", input_str),
                "-thumbnail",
                "256x256>",
                "-background",
                "black",
                "-gravity",
                "center",
                "-extent",
                "256x256",
                "-quality",
                "85",
                output_str,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if matches!(magick_result, Ok(s) if s.success()) {
            return true;
        }
    }

    false
}

/// Returns the web preview file extension if this format is not browser-native.
/// Images → "jpg" (ICO → "png"), Audio → "mp3", Videos → "mp4".
pub fn needs_web_preview(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        // Images that browsers cannot display natively
        "heic" | "heif" | "tiff" | "tif" | "hdr" | "cr2" | "cur" | "cursor"
        | "dng" | "nef" | "arw" | "raw" => Some("jpg"),
        // SVG is browser-native (<img> renders it natively) — no conversion needed.
        // ICO → PNG for consistent rendering on all platforms
        "ico" => Some("png"),
        // Audio that browsers cannot play natively
        "wma" | "aiff" | "aif" => Some("mp3"),
        // Video containers that browsers cannot play natively
        "mkv" | "avi" | "wmv" | "asf" | "h264"
        | "mpg" | "mpeg" | "3gp" | "mov" | "m4v" => Some("mp4"),
        _ => None,
    }
}

/// Public wrapper for background conversion task.
pub async fn generate_web_preview_bg(input_path: &Path, output_path: &Path, preview_ext: &str) -> bool {
    generate_web_preview(input_path, output_path, preview_ext).await
}

/// Generate a browser-compatible web preview file.
/// Images → high-quality JPEG, ICO → PNG, Audio → MP3,
/// Video → MP4 (H.264/AAC).
/// Video conversion uses low-priority CPU settings to avoid starving other tasks.
async fn generate_web_preview(input_path: &Path, output_path: &Path, preview_ext: &str) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let input_str = input_path.to_str().unwrap_or("");
    let output_str = output_path.to_str().unwrap_or("");

    let ffmpeg_ok = match preview_ext {
        "jpg" => {
            // Convert image to high-quality JPEG via FFmpeg
            let status = tokio::process::Command::new("nice")
                .args(["-n", "19", "ffmpeg", "-y", "-i", input_str, "-q:v", "2", output_str])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            matches!(status, Ok(s) if s.success())
        }
        "png" => {
            // ICO → rasterized PNG via FFmpeg (or ImageMagick fallback below)
            let status = tokio::process::Command::new("nice")
                .args(["-n", "19", "ffmpeg", "-y", "-i", input_str, output_str])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            matches!(status, Ok(s) if s.success())
        }
        "mp3" => {
            // Convert audio to MP3
            let status = tokio::process::Command::new("nice")
                .args([
                    "-n", "19", "ffmpeg",
                    "-y",
                    "-i", input_str,
                    "-codec:a", "libmp3lame",
                    "-b:a", "192k",
                    output_str,
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            matches!(status, Ok(s) if s.success())
        }
        "mp4" => {
            // Transcode video to H.264/AAC MP4 — uses nice for low CPU priority.
            // "fast" preset balances quality/speed well for web previews.
            let mut args = vec!["-n", "19", "ffmpeg", "-y"];

            // For raw video streams (.hevc, .h264, .h265), force the demuxer
            // since they lack a container with timing info
            let ext_lower = input_path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            match ext_lower.as_str() {
                "hevc" | "h265" => args.extend_from_slice(&["-f", "hevc"]),
                "h264" => args.extend_from_slice(&["-f", "h264"]),
                _ => {}
            }

            args.extend_from_slice(&[
                "-i", input_str,
                "-c:v", "libx264",
                "-preset", "fast",
                "-crf", "23",
                "-c:a", "aac",
                "-b:a", "128k",
                "-movflags", "+faststart",
                output_str,
            ]);

            let status = tokio::process::Command::new("nice")
                .args(&args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            matches!(status, Ok(s) if s.success())
        }
        _ => false,
    };

    if ffmpeg_ok {
        return true;
    }

    // FFmpeg failed — try ImageMagick as fallback for image/SVG conversions
    if preview_ext == "jpg" || preview_ext == "png" {
        let magick_result = tokio::process::Command::new("convert")
            .args([input_str, "-quality", "92", output_str])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if matches!(magick_result, Ok(s) if s.success()) {
            return true;
        }
    }

    false
}

/// POST /api/admin/photos/scan
/// Scan the storage directory and register all unregistered media files as plain photos.
/// This is the main "import" mechanism for plain mode.
pub async fn scan_and_register(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify admin
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let storage_root = state.storage_root.read().await.clone();

    // Check FFmpeg availability for thumbnail/preview generation
    let has_ffmpeg = ffmpeg_available().await;
    if !has_ffmpeg {
        tracing::warn!("FFmpeg not found — thumbnails and web previews will not be generated");
    }

    // Get already-registered file paths
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT file_path FROM photos WHERE user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;
    let existing_set: std::collections::HashSet<String> = existing.into_iter().collect();

    // Scan recursively for media files
    let mut new_count = 0i64;
    let mut queue = vec![storage_root.clone()];

    while let Some(dir) = queue.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() && is_media_file(&name) {
                    let abs_path = entry.path();
                    // Normalize to forward slashes so DB paths are consistent across OS
                    let rel_path = abs_path
                        .strip_prefix(&storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    if existing_set.contains(&rel_path) {
                        continue; // Already registered
                    }

                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            dt.to_rfc3339()
                        })
                    });

                    let mime = mime_from_extension(&name).to_string();
                    let media_type = if mime.starts_with("video/") {
                        "video"
                    } else if mime.starts_with("audio/") {
                        "audio"
                    } else if mime == "image/gif" {
                        "gif"
                    } else {
                        "photo"
                    };

                    let photo_id = Uuid::new_v4().to_string();
                    let now = Utc::now().to_rfc3339();
                    let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

                    // Extract dimensions, camera model, GPS, and date from file
                    let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
                        extract_media_metadata(&abs_path);

                    // Use EXIF taken_at if available, otherwise fall back to file modified time
                    let final_taken_at = exif_taken.or(modified);

                    // Compute content-based hash using streaming I/O (avoids loading entire file into memory)
                    let photo_hash = compute_photo_hash_streaming(&abs_path).await;

                    sqlx::query(
                        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                         size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, created_at, photo_hash) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(&photo_id)
                    .bind(&auth.user_id)
                    .bind(&name)
                    .bind(&rel_path)
                    .bind(&mime)
                    .bind(media_type)
                    .bind(size)
                    .bind(img_w)
                    .bind(img_h)
                    .bind(&final_taken_at)
                    .bind(exif_lat)
                    .bind(exif_lon)
                    .bind(&cam_model)
                    .bind(&thumb_rel)
                    .bind(&now)
                    .bind(&photo_hash)
                    .execute(&state.pool)
                    .await?;

                    // Generate thumbnail using FFmpeg (or image crate for audio)
                    if has_ffmpeg || mime.starts_with("audio/") {
                        let thumb_abs = storage_root.join(&thumb_rel);
                        if generate_thumbnail_file(&abs_path, &thumb_abs, &mime, None).await {
                            tracing::debug!(file = %rel_path, "Generated thumbnail");
                        } else {
                            tracing::warn!(file = %rel_path, "Failed to generate thumbnail");
                        }
                    }

                    // Generate web preview for non-browser-native formats
                    // Skip video transcoding during scan (too slow) — background task handles it
                    if has_ffmpeg {
                        if let Some(ext) = needs_web_preview(&name) {
                            if ext != "mp4" {
                                let preview_rel =
                                    format!(".web_previews/{}.web.{}", photo_id, ext);
                                let preview_abs = storage_root.join(&preview_rel);
                                if generate_web_preview(&abs_path, &preview_abs, ext).await {
                                    tracing::debug!(file = %rel_path, "Generated web preview");
                                } else {
                                    tracing::warn!(file = %rel_path, "Failed to generate web preview");
                                }
                            } else {
                                tracing::debug!(file = %rel_path, "Video conversion deferred to background task");
                            }
                        }
                    }

                    new_count += 1;
                }
            }
        }
    }

    tracing::info!("Scan complete: registered {} new photos", new_count);

    // ── Retroactively fill missing metadata for existing photos ──────────
    // Fix photos with 0×0 dimensions or missing camera_model/GPS/photo_hash
    let photos_needing_fix: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, file_path FROM photos WHERE user_id = ? AND (width = 0 OR height = 0 OR camera_model IS NULL OR photo_hash IS NULL)",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut fixed_count = 0i64;
    for (pid, fpath) in &photos_needing_fix {
        let abs = storage_root.join(fpath);
        if !abs.exists() {
            continue;
        }
        let (w, h, cam, lat, lon, taken) = extract_media_metadata(&abs);

        // Compute content hash if missing (streaming to avoid loading huge files into memory)
        let file_hash = compute_photo_hash_streaming(&abs).await;

        if w > 0 || h > 0 || cam.is_some() || lat.is_some() || file_hash.is_some() {
            sqlx::query(
                "UPDATE photos SET width = CASE WHEN width = 0 THEN ? ELSE width END, \
                 height = CASE WHEN height = 0 THEN ? ELSE height END, \
                 camera_model = COALESCE(camera_model, ?), \
                 latitude = COALESCE(latitude, ?), \
                 longitude = COALESCE(longitude, ?), \
                 taken_at = COALESCE(taken_at, ?), \
                 photo_hash = COALESCE(photo_hash, ?) \
                 WHERE id = ?",
            )
            .bind(w)
            .bind(h)
            .bind(&cam)
            .bind(lat)
            .bind(lon)
            .bind(&taken)
            .bind(&file_hash)
            .bind(pid)
            .execute(&state.pool)
            .await
            .map_err(|e| {
                tracing::warn!(photo_id = %pid, error = %e, "Failed to update photo metadata during scan");
                e
            })
            .ok();
            fixed_count += 1;
        }
    }

    if fixed_count > 0 {
        tracing::info!("Updated metadata for {} existing photos", fixed_count);
    }

    // ── Generate missing thumbnails and web previews for existing photos ──
    if has_ffmpeg {
        let thumbs_to_gen: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT id, file_path, thumb_path, mime_type FROM photos WHERE user_id = ? AND thumb_path IS NOT NULL",
        )
        .bind(&auth.user_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        let mut thumb_count = 0i64;
        let mut preview_count = 0i64;
        for (pid, fpath, tpath, mime) in &thumbs_to_gen {
            let abs = storage_root.join(fpath);
            if !abs.exists() {
                continue;
            }

            // Generate thumbnail if file doesn't exist yet
            let thumb_abs = storage_root.join(tpath);
            if !thumb_abs.exists() {
                if generate_thumbnail_file(&abs, &thumb_abs, mime, None).await {
                    thumb_count += 1;
                }
            }

            // Generate web preview if needed and doesn't exist yet
            // Skip video transcoding — background task handles those
            let filename = fpath.rsplit('/').next().unwrap_or(fpath);
            if let Some(ext) = needs_web_preview(filename) {
                if ext != "mp4" {
                    let preview_abs =
                        storage_root.join(format!(".web_previews/{}.web.{}", pid, ext));
                    if !preview_abs.exists() {
                        if generate_web_preview(&abs, &preview_abs, ext).await {
                            preview_count += 1;
                        }
                    }
                }
            }
        }

        if thumb_count > 0 {
            tracing::info!("Generated {} missing thumbnails", thumb_count);
        }
        if preview_count > 0 {
            tracing::info!("Generated {} missing web previews", preview_count);
        }
    }

    // Trigger background conversion task — but only if no encryption
    // migration is in progress.  During encryption, conversion would waste
    // work on files that are about to be encrypted (and thus served via
    // blobs, not disk previews).  The migration-done handler will trigger
    // conversion once encryption finishes.
    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "idle".to_string());

    if mig_status == "idle" {
        tracing::info!("[DIAG:SCAN] post-scan: mig_status=idle, sending convert notify");
        state.convert_notify.notify_one();
    } else {
        tracing::info!("[DIAG:SCAN] post-scan: mig_status='{}', skipping conversion trigger", mig_status);
    }

    Ok(Json(serde_json::json!({
        "registered": new_count,
        "metadata_updated": fixed_count,
        "message": format!("{} new photos registered, {} metadata updated", new_count, fixed_count),
    })))
}
