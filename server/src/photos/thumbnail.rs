//! Thumbnail generation for media files.
//!
//! Supports multiple formats with a priority-based fallback chain:
//! - **Images** (non-GIF): Pure Rust `image` crate → fit within 512px JPEG.
//! - **GIFs**: FFmpeg → scaled animated GIF; falls back to static single-frame.
//! - **Videos**: FFmpeg → real frame at ~10% of duration; falls back to placeholder.
//! - **Audio**: Solid-color placeholder (no external tools needed).
//!
//! Thumbnails preserve the original aspect ratio (scaled to fit within 512px
//! on the longest edge) so the justified grid can display them without
//! excessive cropping.
//!
//! **EXIF orientation** is applied during thumbnail generation for static
//! images so that camera-taken portrait photos render correctly.  The
//! `image` crate's `open()` loads raw pixel data without consulting EXIF,
//! so we read the orientation tag separately and apply the matching
//! rotation/flip before resizing.
//!
//! Extracted from `scan.rs` so that upload, backup, and migration code can
//! reuse the same thumbnail pipeline without pulling in the scan handler.

use std::path::Path;

use crate::process::{run_with_timeout, status_with_timeout, THUMBNAIL_TIMEOUT, FFPROBE_TIMEOUT};

/// Generate a thumbnail for a media file.
///
/// Supported inputs: JPEG, PNG, GIF, WebP, BMP, ICO, AVIF, video, audio.
///
/// The `output_path` extension determines the format:
/// - `.thumb.gif` for animated GIF thumbnails
/// - `.thumb.jpg` for everything else
pub async fn generate_thumbnail_file(
    input_path: &Path,
    output_path: &Path,
    mime: &str,
    _crop_metadata: Option<&str>,
) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    // Audio → black 256×256 placeholder with no external tools
    if mime.starts_with("audio/") {
        return generate_placeholder_thumbnail(output_path, [0, 0, 0]).await;
    }

    // Video → FFmpeg frame extraction, fallback to placeholder
    if mime.starts_with("video/") {
        return generate_video_thumbnail_ffmpeg(input_path, output_path).await;
    }

    // GIF → FFmpeg scaled animated GIF, fallback to static single-frame GIF
    if mime == "image/gif" {
        if generate_gif_thumbnail_ffmpeg(input_path, output_path).await {
            return true;
        }
        tracing::debug!(path = %input_path.display(), "FFmpeg GIF thumbnail failed, falling back to static GIF");
        return generate_static_gif_thumbnail(input_path, output_path).await;
    }

    // All other image formats → image crate (JPEG)
    generate_static_image_thumbnail(input_path, output_path).await
}

/// Generate a JPEG thumbnail from a static image using the `image` crate.
///
/// Preserves the original aspect ratio, scaling so the longest edge is at
/// most 512 px.  This avoids the double-crop problem where a square
/// thumbnail displayed in an aspect-ratio-preserving grid causes excessive
/// zoom / cut-off.
async fn generate_static_image_thumbnail(input_path: &Path, output_path: &Path) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let input = input_path.to_path_buf();
    let output = output_path.to_path_buf();

    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let img = image::open(&input).map_err(|e| format!("Failed to open image: {}", e))?;
        let img = apply_exif_orientation(&input, img);
        let thumb = img.resize(512, 512, image::imageops::FilterType::Triangle);
        thumb
            .save_with_format(&output, image::ImageFormat::Jpeg)
            .map_err(|e| format!("Failed to save thumbnail: {}", e))?;
        Ok(())
    })
    .await;

    match result {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            tracing::warn!(path = %input_path.display(), error = %e, "Thumbnail generation failed");
            generate_placeholder_thumbnail(output_path, [50, 50, 50]).await
        }
        Err(e) => {
            tracing::warn!(path = %input_path.display(), error = %e, "Thumbnail task panicked");
            false
        }
    }
}

/// Generate a single-frame GIF thumbnail as a fallback when FFmpeg is
/// unavailable.  Preserves aspect ratio (fits within 512px).
/// Writes to the same `.thumb.gif` path the DB references.
async fn generate_static_gif_thumbnail(input_path: &Path, output_path: &Path) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let input = input_path.to_path_buf();
    let output = output_path.to_path_buf();

    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let img = image::open(&input).map_err(|e| format!("Failed to open GIF: {}", e))?;
        let thumb = img.resize(512, 512, image::imageops::FilterType::Triangle);
        thumb
            .save_with_format(&output, image::ImageFormat::Gif)
            .map_err(|e| format!("Failed to save GIF thumbnail: {}", e))?;
        Ok(())
    })
    .await;

    match result {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            tracing::warn!(path = %input_path.display(), error = %e, "Static GIF thumbnail failed");
            generate_placeholder_thumbnail(output_path, [50, 50, 50]).await
        }
        Err(e) => {
            tracing::warn!(path = %input_path.display(), error = %e, "GIF thumbnail task panicked");
            false
        }
    }
}

/// Extract a real video frame using FFmpeg and save as a JPEG thumbnail.
///
/// Preserves the original aspect ratio, scaling so the longest edge is at
/// most 512 px.  This avoids the double-crop that occurred when a square
/// thumbnail was displayed in the aspect-ratio-preserving justified grid.
///
/// Seeks to 10% of the video duration (at least 1 second in) to avoid
/// black intro frames.  For very short clips (≤ 1.5 s) it grabs the
/// first frame instead.  Falls back to a gray placeholder if FFmpeg fails.
async fn generate_video_thumbnail_ffmpeg(input_path: &Path, output_path: &Path) -> bool {
    let duration_secs = probe_duration(input_path).await.unwrap_or(10.0);
    let seek_to = if duration_secs <= 1.5 {
        0.0
    } else {
        f64::min(f64::max(duration_secs * 0.1, 1.0), duration_secs - 0.5)
    };

    // setsar=1 normalises non-square pixel aspect ratios before scaling.
    // force_original_aspect_ratio=decrease fits within 512×512 without cropping.
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.args(["-y", "-ss", &format!("{:.2}", seek_to), "-i"])
        .arg(input_path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "setsar=1,scale=512:512:force_original_aspect_ratio=decrease",
            "-q:v",
            "5",
        ])
        .arg(output_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let result = status_with_timeout(&mut cmd, THUMBNAIL_TIMEOUT).await;

    match result {
        Ok(status) if status.success() && output_path.exists() => {
            tracing::debug!(path = %input_path.display(), "Generated video thumbnail via FFmpeg");
            true
        }
        Ok(status) => {
            tracing::warn!(
                path = %input_path.display(),
                exit_code = ?status.code(),
                "FFmpeg video thumbnail failed, using placeholder"
            );
            generate_placeholder_thumbnail(output_path, [30, 30, 30]).await
        }
        Err(e) => {
            tracing::warn!(
                path = %input_path.display(),
                error = %e,
                "FFmpeg not available or timed out for video thumbnail, using placeholder"
            );
            generate_placeholder_thumbnail(output_path, [30, 30, 30]).await
        }
    }
}

/// Generate a scaled animated GIF thumbnail using FFmpeg.
///
/// Preserves the original aspect ratio (fits within 512px) at reduced
/// frame rate to keep file size reasonable.
async fn generate_gif_thumbnail_ffmpeg(input_path: &Path, output_path: &Path) -> bool {
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.args(["-y", "-i"])
        .arg(input_path)
        .args([
            "-vf",
            "scale=512:512:force_original_aspect_ratio=decrease,fps=15",
            "-loop",
            "0",
        ])
        .arg(output_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let result = status_with_timeout(&mut cmd, THUMBNAIL_TIMEOUT).await;

    match result {
        Ok(status) if status.success() && output_path.exists() => {
            tracing::debug!(path = %input_path.display(), "Generated animated GIF thumbnail via FFmpeg");
            true
        }
        _ => false,
    }
}

/// Probe the duration of a media file using ffprobe.
pub async fn probe_duration(path: &Path) -> Option<f64> {
    let mut cmd = tokio::process::Command::new("ffprobe");
    cmd.args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let output = run_with_timeout(&mut cmd, FFPROBE_TIMEOUT).await.ok()?;

    let s = String::from_utf8_lossy(&output.stdout);
    s.trim().parse::<f64>().ok()
}

/// Generate a solid-color 512×512 JPEG placeholder thumbnail.
pub async fn generate_placeholder_thumbnail(output_path: &Path, color: [u8; 3]) -> bool {
    let img = image::RgbImage::from_pixel(512, 512, image::Rgb(color));
    let mut buf = std::io::Cursor::new(Vec::new());
    if image::DynamicImage::ImageRgb8(img)
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .is_ok()
    {
        return tokio::fs::write(output_path, buf.into_inner())
            .await
            .is_ok();
    }
    false
}

/// Read the EXIF orientation tag from an image file and apply the
/// corresponding rotation/flip to the `DynamicImage`.
///
/// EXIF orientation values:
/// 1 = Normal
/// 2 = Flip horizontal
/// 3 = Rotate 180°
/// 4 = Flip vertical
/// 5 = Rotate 90° CW + flip horizontal
/// 6 = Rotate 90° CW       (most common portrait)
/// 7 = Rotate 90° CCW + flip horizontal
/// 8 = Rotate 90° CCW
pub(super) fn apply_exif_orientation(path: &Path, img: image::DynamicImage) -> image::DynamicImage {
    let orientation = (|| -> Option<u32> {
        let file = std::fs::File::open(path).ok()?;
        let mut reader = std::io::BufReader::new(file);
        let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
        let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
        field.value.get_uint(0)
    })();

    match orientation {
        Some(2) => img.fliph(),
        Some(3) => img.rotate180(),
        Some(4) => img.flipv(),
        Some(5) => img.rotate90().fliph(),
        Some(6) => img.rotate90(),
        Some(7) => img.rotate270().fliph(),
        Some(8) => img.rotate270(),
        _ => img, // 1 or missing — no rotation needed
    }
}

/// Apply EXIF orientation from raw image bytes (in-memory variant).
///
/// Same rotation/flip logic as [`apply_exif_orientation`] but reads the
/// EXIF tag from a byte slice instead of a file path.  Used by the
/// encryption migration pipeline where the file data is already in memory.
pub(super) fn apply_exif_orientation_from_bytes(data: &[u8], img: image::DynamicImage) -> image::DynamicImage {
    let orientation = (|| -> Option<u32> {
        let mut cursor = std::io::Cursor::new(data);
        let exif = exif::Reader::new().read_from_container(&mut cursor).ok()?;
        let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
        field.value.get_uint(0)
    })();

    match orientation {
        Some(2) => img.fliph(),
        Some(3) => img.rotate180(),
        Some(4) => img.flipv(),
        Some(5) => img.rotate90().fliph(),
        Some(6) => img.rotate90(),
        Some(7) => img.rotate270().fliph(),
        Some(8) => img.rotate270(),
        _ => img,
    }
}

/// Read the EXIF orientation value from a file (0 if unreadable or absent).
fn read_exif_orientation(path: &Path) -> u32 {
    (|| -> Option<u32> {
        let file = std::fs::File::open(path).ok()?;
        let mut reader = std::io::BufReader::new(file);
        let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
        let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
        field.value.get_uint(0)
    })()
    .unwrap_or(0)
}

/// One-time startup task: regenerate thumbnails for photos whose source
/// image has an EXIF orientation ≥ 2 (i.e. needs rotation/flip).
///
/// Previous thumbnail generation did **not** apply EXIF orientation, so
/// portrait camera photos had landscape thumbnails. This task corrects
/// existing thumbnails and records a flag so it only runs once.
pub async fn repair_thumbnail_orientation(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
) {
    // Check one-time flag
    let done: Option<String> = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'thumb_orientation_repaired'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    if done.is_some() {
        return;
    }

    // Fetch all non-GIF image photos that have a file_path and thumb_path
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, file_path, thumb_path FROM photos \
         WHERE file_path != '' AND thumb_path IS NOT NULL \
         AND media_type = 'photo' AND mime_type != 'image/gif'",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let total = rows.len();
    if total == 0 {
        tracing::info!("[THUMB-REPAIR] No photos to check");
    } else {
        let mut repaired = 0u64;
        for (id, file_path, thumb_path) in &rows {
            let src = storage_root.join(file_path);
            let orient = tokio::task::spawn_blocking({
                let src = src.clone();
                move || read_exif_orientation(&src)
            })
            .await
            .unwrap_or(0);

            if orient >= 2 {
                let thumb_abs = storage_root.join(thumb_path);
                if generate_thumbnail_file(&src, &thumb_abs, "image/jpeg", None).await {
                    repaired += 1;
                    tracing::debug!(photo_id = %id, orientation = orient, "[THUMB-REPAIR] Regenerated thumbnail");
                }
            }
        }
        tracing::info!(
            "[THUMB-REPAIR] Checked {} photos, regenerated {} thumbnails",
            total,
            repaired
        );
    }

    // Record flag
    let _ = sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('thumb_orientation_repaired', '1') \
         ON CONFLICT(key) DO UPDATE SET value = '1'",
    )
    .execute(pool)
    .await;
}
