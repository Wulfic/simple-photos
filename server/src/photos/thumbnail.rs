//! Thumbnail generation for media files.
//!
//! Supports multiple formats with a priority-based fallback chain:
//! - **Images** (non-GIF): Pure Rust `image` crate → 512×512 JPEG.
//! - **GIFs**: FFmpeg → scaled animated GIF; falls back to static single-frame.
//! - **Videos**: FFmpeg → real frame at ~10% of duration; falls back to placeholder.
//! - **Audio / SVG**: Solid-color placeholder (no external tools needed).
//!
//! Extracted from `scan.rs` so that upload, backup, and migration code can
//! reuse the same thumbnail pipeline without pulling in the scan handler.

use std::path::Path;

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

    // SVG → placeholder (would need resvg for proper rasterisation)
    if mime == "image/svg+xml" {
        return generate_placeholder_thumbnail(output_path, [40, 40, 40]).await;
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

/// Generate a 512×512 JPEG thumbnail from a static image using the `image` crate.
async fn generate_static_image_thumbnail(input_path: &Path, output_path: &Path) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let input = input_path.to_path_buf();
    let output = output_path.to_path_buf();

    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let img = image::open(&input).map_err(|e| format!("Failed to open image: {}", e))?;
        let thumb = img.resize_to_fill(512, 512, image::imageops::FilterType::Triangle);
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

/// Generate a 512×512 single-frame GIF thumbnail as a fallback when FFmpeg is
/// unavailable.  Writes to the same `.thumb.gif` path the DB references.
async fn generate_static_gif_thumbnail(input_path: &Path, output_path: &Path) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let input = input_path.to_path_buf();
    let output = output_path.to_path_buf();

    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let img = image::open(&input).map_err(|e| format!("Failed to open GIF: {}", e))?;
        let thumb = img.resize_to_fill(512, 512, image::imageops::FilterType::Triangle);
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

/// Extract a real video frame using FFmpeg and save as 256×256 JPEG thumbnail.
///
/// Seeks to 10% of the video duration (at least 1 second in) to avoid
/// black intro frames. Falls back to a gray placeholder if FFmpeg fails.
async fn generate_video_thumbnail_ffmpeg(input_path: &Path, output_path: &Path) -> bool {
    let duration_secs = probe_duration(input_path).await.unwrap_or(10.0);
    let seek_to = f64::min(f64::max(duration_secs * 0.1, 1.0), duration_secs);

    // setsar=1 normalises non-square pixel aspect ratios before scaling
    // so the thumbnail isn't stretched/squished for converted videos.
    let result = tokio::process::Command::new("ffmpeg")
        .args(["-y", "-ss", &format!("{:.2}", seek_to), "-i"])
        .arg(input_path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "setsar=1,scale=512:512:force_original_aspect_ratio=increase,crop=512:512",
            "-q:v",
            "5",
        ])
        .arg(output_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

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
                "FFmpeg not available for video thumbnail, using placeholder"
            );
            generate_placeholder_thumbnail(output_path, [30, 30, 30]).await
        }
    }
}

/// Generate a scaled animated GIF thumbnail using FFmpeg.
///
/// Produces a 256×256 (cover-cropped) animated GIF at reduced frame rate
/// to keep file size reasonable.
async fn generate_gif_thumbnail_ffmpeg(input_path: &Path, output_path: &Path) -> bool {
    let result = tokio::process::Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(input_path)
        .args([
            "-vf",
            "scale=512:512:force_original_aspect_ratio=increase,crop=512:512,fps=15",
            "-loop",
            "0",
        ])
        .arg(output_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

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
    let output = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .ok()?;

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
