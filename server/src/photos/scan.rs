//! Filesystem scanning, thumbnail generation, and web preview creation.
//!
//! `POST /api/admin/photos/scan` walks the storage directory tree, registers
//! every unregistered media file as a plain photo, extracts EXIF metadata,
//! generates JPEG thumbnails (via ImageMagick → FFmpeg fallback), and creates
//! browser-compatible web previews for non-native image/audio formats.
//!
//! **Video transcoding** (MKV → MP4, AVI → MP4, etc.) is intentionally deferred
//! to the background processing pipeline in [`super::convert`].  That pipeline
//! runs three sequential phases after scan completes:
//!   1. Generate thumbnails for ALL files
//!   2. Convert flagged files to browser-friendly formats
//!   3. Regenerate thumbnails for freshly converted files only
//!
//! **Original preservation**: Conversion never deletes the original file.
//! The converted copy is stored separately in `.web_previews/`.  Both the
//! original (`/photos/:id/file`) and the web-friendly version
//! (`/photos/:id/web`) remain available for download.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use axum::extract::State;
use axum::Json;
use futures_util::TryStreamExt;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::metadata::extract_media_metadata_async;
use super::utils::{normalize_iso_timestamp, utc_now_iso, compute_photo_hash_streaming};

// compute_photo_hash_streaming is now in utils.rs — imported above.

/// Maximum concurrent file processing tasks during scan.
/// Bounded to avoid fork-bombing the system with FFmpeg/ImageMagick subprocesses.
const SCAN_PARALLELISM: usize = 4;

/// Check if FFmpeg is available on the system (public for background convert task).
pub async fn ffmpeg_available_pub() -> bool {
    ffmpeg_available().await
}

/// Check if FFmpeg is available on the system.
async fn ffmpeg_available() -> bool {
    tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .stdin(std::process::Stdio::null())
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
            .stdin(std::process::Stdio::null())
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
    cmd.stdin(std::process::Stdio::null());
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
            let rotate = meta.get("rotate").and_then(|v| v.as_f64()).unwrap_or(0.0);

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

            // Apply rotation if non-zero
            if rotate != 0.0 {
                // FFmpeg rotate filter takes radians. Positive is clockwise.
                vf_filters.push_str(&format!("rotate={}*PI/180:ow='rotw({}*PI/180)':oh='roth({}*PI/180)',", rotate, rotate, rotate));
            }
        }
    }

    // Normalize sample aspect ratio (SAR) before scaling.  Many .3gp and
    // other legacy video files use non-square pixels (anamorphic encoding).
    // Without this, the 256×256 scale produces a squished thumbnail because
    // it operates on raw pixel dimensions rather than display dimensions.
    if mime.starts_with("video/") {
        vf_filters.push_str("scale=iw*sar:ih,setsar=1,");
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
            .stdin(std::process::Stdio::null())
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
        // SVG → rasterized 1080p JPEG for consistent cross-platform rendering.
        // Browsers can render SVG natively but Android Coil and many image
        // viewers struggle with vector content.  A high-resolution JPEG
        // guarantees reliable display everywhere.
        "svg" => Some("jpg"),
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

    // SVG files need ImageMagick for reliable rasterisation — FFmpeg cannot
    // handle vector inputs.  Rasterise to 1080p JPEG up front so we skip the
    // generic FFmpeg path entirely.
    let is_svg = input_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);
    if is_svg && preview_ext == "jpg" {
        // Try ImageMagick: rasterise at 1920×1080 max (preserving aspect ratio)
        let magick_ok = tokio::process::Command::new("convert")
            .args([
                &format!("{}[0]", input_str), // first layer only
                "-background", "white",       // SVGs often have transparent bg
                "-flatten",
                "-resize", "1920x1080>",       // scale down only, keep aspect
                "-quality", "92",
                output_str,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if matches!(magick_ok, Ok(s) if s.success()) {
            return true;
        }
        // ImageMagick failed — return false, nothing else can rasterise SVG
        return false;
    }

    let ffmpeg_ok = match preview_ext {
        "jpg" => {
            // Convert image to high-quality JPEG via FFmpeg
            let status = tokio::process::Command::new("nice")
                .args(["-n", "19", "ffmpeg", "-y", "-i", input_str, "-q:v", "2", output_str])
                .stdin(std::process::Stdio::null())
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
                .stdin(std::process::Stdio::null())
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
                .stdin(std::process::Stdio::null())
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
                .stdin(std::process::Stdio::null())
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
            .stdin(std::process::Stdio::null())
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
///
/// For each new file: extracts EXIF metadata, generates a thumbnail, and
/// creates web previews for non-native image/audio formats.  Video transcoding
/// (MKV → MP4, AVI → MP4, etc.) is deferred to the background processing
/// pipeline in [`super::convert`] which handles it after all thumbnails are
/// generated.
///
/// Uses `INSERT OR IGNORE` for graceful handling of concurrent scans (e.g.
/// the background autoscan running simultaneously).
///
/// Original files are **never modified or deleted** by this endpoint.
pub async fn scan_and_register(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    // Serialize scan operations to prevent concurrent scans from racing.
    // The UNIQUE(user_id, file_path) constraint is the safety net, but the
    // mutex avoids wasted work (duplicate metadata extraction, thumbnail gen).
    let _scan_guard = state.scan_lock.lock().await;

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();

    // Check FFmpeg availability for thumbnail/preview generation
    let has_ffmpeg = ffmpeg_available().await;
    if !has_ffmpeg {
        tracing::warn!("FFmpeg not found — thumbnails and web previews will not be generated");
    }

    // Build set of already-registered paths using a streaming cursor so we
    // never hold the full Vec<String> + HashSet simultaneously in memory.
    let mut existing_set = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM photos WHERE user_id = ?",
        )
        .bind(&auth.user_id)
        .fetch(&state.pool);

        while let Some(path) = rows.try_next().await? {
            existing_set.insert(path);
        }
    }

    // Check audio backup toggle — skip audio files when disabled
    let audio_backup_enabled: bool = sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(true); // default: enabled

    // ── Phase 1: Collect all unregistered media files (fast directory walk) ──
    // We collect candidates first without any heavy I/O so the walk is fast.
    struct ScanCandidate {
        abs_path: PathBuf,
        rel_path: String,
        name: String,
        mime: String,
        media_type: &'static str,
        size: i64,
        modified: Option<String>,
    }

    let mut candidates: Vec<ScanCandidate> = Vec::new();
    let mut skipped_audio = 0i64;
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
                    let rel_path = abs_path
                        .strip_prefix(&storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    if existing_set.contains(&rel_path) {
                        continue;
                    }

                    let mime = mime_from_extension(&name).to_string();
                    let media_type: &'static str = if mime.starts_with("video/") {
                        "video"
                    } else if mime.starts_with("audio/") {
                        "audio"
                    } else if mime == "image/gif" {
                        "gif"
                    } else {
                        "photo"
                    };

                    if media_type == "audio" && !audio_backup_enabled {
                        skipped_audio += 1;
                        continue;
                    }

                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            normalize_iso_timestamp(&dt.to_rfc3339())
                        })
                    });

                    candidates.push(ScanCandidate {
                        abs_path,
                        rel_path,
                        name,
                        mime,
                        media_type,
                        size,
                        modified,
                    });
                }
            }
        }
    }

    tracing::info!("Scan phase 1: found {} unregistered media files", candidates.len());

    // ── Phase 2: Register files in parallel (metadata, hash, DB insert, thumbnail) ──
    let new_count = Arc::new(AtomicI64::new(0));
    let sem = Arc::new(Semaphore::new(SCAN_PARALLELISM));
    let mut handles = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let sem = sem.clone();
        let new_count = new_count.clone();
        let pool = state.pool.clone();
        let storage_root = storage_root.clone();
        let user_id = auth.user_id.clone();
        let has_ffmpeg = has_ffmpeg;

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;

            let photo_id = Uuid::new_v4().to_string();
            let now = utc_now_iso();
            let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

            // Extract dimensions, camera model, GPS, and date from file
            let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
                extract_media_metadata_async(candidate.abs_path.clone()).await;

            let final_taken_at = exif_taken
                .map(|t| normalize_iso_timestamp(&t))
                .or(candidate.modified);

            // Compute content-based hash using streaming I/O
            let photo_hash = compute_photo_hash_streaming(&candidate.abs_path).await;

            let insert_result = sqlx::query(
                "INSERT OR IGNORE INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                 size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, created_at, photo_hash) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&photo_id)
            .bind(&user_id)
            .bind(&candidate.name)
            .bind(&candidate.rel_path)
            .bind(&candidate.mime)
            .bind(candidate.media_type)
            .bind(candidate.size)
            .bind(img_w)
            .bind(img_h)
            .bind(&final_taken_at)
            .bind(exif_lat)
            .bind(exif_lon)
            .bind(&cam_model)
            .bind(&thumb_rel)
            .bind(&now)
            .bind(&photo_hash)
            .execute(&pool)
            .await;

            match insert_result {
                Ok(result) if result.rows_affected() == 0 => {
                    tracing::debug!(file = %candidate.rel_path, "Already registered (concurrent scan), skipping");
                    return;
                }
                Err(e) => {
                    tracing::error!(file = %candidate.rel_path, error = %e, "Failed to register photo");
                    return;
                }
                Ok(_) => {}
            }

            // Generate thumbnail
            if has_ffmpeg || candidate.mime.starts_with("audio/") {
                let thumb_abs = storage_root.join(&thumb_rel);
                if generate_thumbnail_file(&candidate.abs_path, &thumb_abs, &candidate.mime, None).await {
                    tracing::debug!(file = %candidate.rel_path, "Generated thumbnail");
                } else {
                    tracing::warn!(file = %candidate.rel_path, "Failed to generate thumbnail");
                }
            }

            // Generate web preview for non-browser-native formats (skip video transcoding)
            if has_ffmpeg {
                if let Some(ext) = needs_web_preview(&candidate.name) {
                    if ext != "mp4" {
                        let preview_rel = format!(".web_previews/{}.web.{}", photo_id, ext);
                        let preview_abs = storage_root.join(&preview_rel);
                        if generate_web_preview(&candidate.abs_path, &preview_abs, ext).await {
                            tracing::debug!(file = %candidate.rel_path, "Generated web preview");
                        } else {
                            tracing::warn!(file = %candidate.rel_path, "Failed to generate web preview");
                        }
                    }
                }
            }

            new_count.fetch_add(1, Ordering::Relaxed);
        }));
    }

    // Wait for all registration tasks to complete
    for h in handles {
        let _ = h.await;
    }

    let new_count = new_count.load(Ordering::Relaxed);
    tracing::info!("Scan complete: registered {} new photos (skipped {} audio)", new_count, skipped_audio);

    // ── Retroactively fill missing metadata for existing photos ──────────
    // Fix photos with 0×0 dimensions or missing camera_model/GPS/photo_hash
    let photos_needing_fix: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, file_path FROM photos WHERE user_id = ? AND (width = 0 OR height = 0 OR camera_model IS NULL OR photo_hash IS NULL)",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let fixed_count = Arc::new(AtomicI64::new(0));
    {
        let sem = Arc::new(Semaphore::new(SCAN_PARALLELISM));
        let mut handles = Vec::with_capacity(photos_needing_fix.len());

        for (pid, fpath) in photos_needing_fix {
            let abs = storage_root.join(&fpath);
            if !abs.exists() {
                continue;
            }
            let sem = sem.clone();
            let pool = state.pool.clone();
            let fixed_count = fixed_count.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await;
                let (w, h, cam, lat, lon, taken) = extract_media_metadata_async(abs.clone()).await;
                let file_hash = compute_photo_hash_streaming(&abs).await;

                if w > 0 || h > 0 || cam.is_some() || lat.is_some() || file_hash.is_some() {
                    sqlx::query(
                        "UPDATE OR IGNORE photos SET width = CASE WHEN width = 0 THEN ? ELSE width END, \
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
                    .bind(&pid)
                    .execute(&pool)
                    .await
                    .map_err(|e| {
                        tracing::warn!(photo_id = %pid, error = %e, "Failed to update photo metadata during scan");
                        e
                    })
                    .ok();
                    fixed_count.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }
    }
    let fixed_count = fixed_count.load(Ordering::Relaxed);

    if fixed_count > 0 {
        tracing::info!("Updated metadata for {} existing photos", fixed_count);
    }

    // ── Generate missing thumbnails and web previews for existing photos ──
    // Parallelized using the same semaphore pattern as Phase 2.
    if has_ffmpeg {
        let thumbs_to_gen: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT id, file_path, thumb_path, mime_type FROM photos WHERE user_id = ? AND thumb_path IS NOT NULL",
        )
        .bind(&auth.user_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        // Collect work items that actually need generation
        struct ThumbWork {
            source_abs: PathBuf,
            thumb_abs: Option<PathBuf>,
            preview_abs: Option<PathBuf>,
            mime: String,
            preview_ext: Option<&'static str>,
        }

        let mut work_items: Vec<ThumbWork> = Vec::new();
        for (pid, fpath, tpath, mime) in &thumbs_to_gen {
            let abs = storage_root.join(fpath);
            if !abs.exists() {
                continue;
            }

            let thumb_abs = storage_root.join(tpath);
            let need_thumb = !thumb_abs.exists();

            let filename = fpath.rsplit('/').next().unwrap_or(fpath);
            let (need_preview, preview_abs, preview_ext) = if let Some(ext) = needs_web_preview(filename) {
                if ext != "mp4" {
                    let pa = storage_root.join(format!(".web_previews/{}.web.{}", pid, ext));
                    (!pa.exists(), Some(pa), Some(ext))
                } else {
                    (false, None, None)
                }
            } else {
                (false, None, None)
            };

            if need_thumb || need_preview {
                work_items.push(ThumbWork {
                    source_abs: abs,
                    thumb_abs: if need_thumb { Some(thumb_abs) } else { None },
                    preview_abs: if need_preview { preview_abs } else { None },
                    mime: mime.clone(),
                    preview_ext,
                });
            }
        }

        let thumb_count = Arc::new(AtomicI64::new(0));
        let preview_count = Arc::new(AtomicI64::new(0));
        let sem = Arc::new(Semaphore::new(SCAN_PARALLELISM));
        let mut handles = Vec::with_capacity(work_items.len());

        for item in work_items {
            let sem = sem.clone();
            let tc = thumb_count.clone();
            let pc = preview_count.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await;

                if let Some(thumb_abs) = item.thumb_abs {
                    if generate_thumbnail_file(&item.source_abs, &thumb_abs, &item.mime, None).await {
                        tc.fetch_add(1, Ordering::Relaxed);
                    }
                }

                if let Some(preview_abs) = item.preview_abs {
                    if let Some(ext) = item.preview_ext {
                        if generate_web_preview(&item.source_abs, &preview_abs, ext).await {
                            pc.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let tc = thumb_count.load(Ordering::Relaxed);
        let pc = preview_count.load(Ordering::Relaxed);
        if tc > 0 {
            tracing::info!("Generated {} missing thumbnails", tc);
        }
        if pc > 0 {
            tracing::info!("Generated {} missing web previews", pc);
        }
    }

    // Trigger the background processing pipeline — runs three sequential
    // phases: thumbnails → conversion → post-conversion thumbnails.
    // Original files are always preserved; the pipeline writes converted
    // copies to `.web_previews/` without touching originals.
    tracing::info!("[DIAG:SCAN] post-scan: sending pipeline notify");
    state.convert_notify.notify_one();

    // After registering new photos, check if encryption migration needs to
    // start (for any newly discovered unencrypted files).
    crate::backup::autoscan::try_start_migration_after_scan(
        &state.pool,
        &storage_root,
        &state.convert_notify,
        &state.encryption_key,
        &state.config.auth.jwt_secret,
    ).await;

    Ok(Json(serde_json::json!({
        "registered": new_count,
        "metadata_updated": fixed_count,
        "skipped_audio": skipped_audio,
        "message": format!("{} new photos registered, {} metadata updated", new_count, fixed_count),
    })))
}
