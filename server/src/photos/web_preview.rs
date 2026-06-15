//! Web preview generation for non-browser-native media formats.
//!
//! Some formats (HEIC, MKV, WMA, etc.) can't be displayed natively in a
//! browser. This module converts them into web-friendly formats via FFmpeg.
//! (FFmpeg only — no ImageMagick — to keep the install to a single media tool.
//! HEIC/HEIF decodes natively via FFmpeg's mov demuxer + built-in HEVC decoder,
//! so no libheif build is required.)

use std::path::Path;

/// Check if a file needs a web-compatible preview (i.e. browsers can't render
/// it natively). Returns the target preview extension if conversion is needed.
pub fn needs_web_preview(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        // Images that browsers cannot display natively. (RAW formats — cr2,
        // dng, nef, arw, raw — are intentionally omitted: FFmpeg can't decode
        // them and we no longer ship ImageMagick, so they are unsupported.)
        "heic" | "heif" | "tiff" | "tif" | "hdr" | "cur" | "cursor" => Some("jpg"),
        "ico" => Some("png"),
        // Audio that browsers cannot play natively
        "wma" | "aiff" | "aif" => Some("mp3"),
        // Video containers that browsers cannot play natively
        "mkv" | "avi" | "wmv" | "asf" | "h264" | "mpg" | "mpeg" | "3gp" | "mov" | "m4v" => {
            Some("mp4")
        }
        _ => None,
    }
}

/// Public wrapper for background web preview generation.
pub async fn generate_web_preview_bg(
    input_path: &Path,
    output_path: &Path,
    preview_ext: &str,
) -> bool {
    generate_web_preview(input_path, output_path, preview_ext).await
}

/// Ceiling for a single preview conversion.  Without it a hung FFmpeg
/// (corrupt input, stuck GPU session) wedged the preview task forever.
const PREVIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Generate a browser-compatible web preview file.
/// Images → high-quality JPEG, ICO → PNG, Audio → MP3, Video → MP4 (H.264/AAC).
async fn generate_web_preview(input_path: &Path, output_path: &Path, preview_ext: &str) -> bool {
    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let input_str = input_path.to_str().unwrap_or("");
    let output_str = output_path.to_str().unwrap_or("");

    let ffmpeg_ok = match preview_ext {
        "jpg" => {
            let mut cmd = crate::process::background_command("ffmpeg");
            cmd.args(["-y", "-i", input_str, "-q:v", "2", output_str])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let status = crate::process::status_with_timeout(&mut cmd, PREVIEW_TIMEOUT).await;
            matches!(status, Ok(s) if s.success())
        }
        "png" => {
            let mut cmd = crate::process::background_command("ffmpeg");
            cmd.args(["-y", "-i", input_str, output_str])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let status = crate::process::status_with_timeout(&mut cmd, PREVIEW_TIMEOUT).await;
            matches!(status, Ok(s) if s.success())
        }
        "mp3" => {
            let mut cmd = crate::process::background_command("ffmpeg");
            cmd.args([
                "-y",
                "-i",
                input_str,
                "-codec:a",
                "libmp3lame",
                "-b:a",
                "192k",
                output_str,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
            let status = crate::process::status_with_timeout(&mut cmd, PREVIEW_TIMEOUT).await;
            matches!(status, Ok(s) if s.success())
        }
        "mp4" => {
            // Route through the shared GPU-aware video transcoder so
            // on-the-fly previews honour the NVENC / QSV / VAAPI path
            // configured at startup. Falls back to libx264 internally
            // when no hwaccel is registered or the GPU encode fails.
            let hwaccel = crate::conversion::active_hwaccel();
            let fallback = crate::conversion::cpu_fallback_enabled();
            crate::conversion::convert_video(input_str, output_str, hwaccel, fallback).await
        }
        _ => false,
    };

    if ffmpeg_ok {
        return true;
    }

    tracing::warn!(
        input = %input_path.display(),
        target = preview_ext,
        "Web preview: FFmpeg conversion failed"
    );
    false
}
