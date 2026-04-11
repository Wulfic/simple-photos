//! Web preview generation for non-browser-native media formats.
//!
//! Some formats (HEIC, MKV, WMA, etc.) can't be displayed natively in a
//! browser. This module converts them into web-friendly formats via FFmpeg
//! or ImageMagick, with fallback chains.

use std::path::Path;

/// Check if a file needs a web-compatible preview (i.e. browsers can't render
/// it natively). Returns the target preview extension if conversion is needed.
pub fn needs_web_preview(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        // Images that browsers cannot display natively
        "heic" | "heif" | "tiff" | "tif" | "hdr" | "cr2" | "cur" | "cursor" | "dng" | "nef"
        | "arw" | "raw" => Some("jpg"),
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
            let status = tokio::process::Command::new("nice")
                .args([
                    "-n",
                    "19",
                    "ffmpeg",
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
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            matches!(status, Ok(s) if s.success())
        }
        "mp4" => {
            let mut args = vec!["-n", "19", "ffmpeg", "-y"];
            let ext_lower = input_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            match ext_lower.as_str() {
                "hevc" | "h265" => args.extend_from_slice(&["-f", "hevc"]),
                "h264" => args.extend_from_slice(&["-f", "h264"]),
                _ => {}
            }
            args.extend_from_slice(&[
                "-i",
                input_str,
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-crf",
                "23",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
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

    // FFmpeg failed — try ImageMagick as fallback for image conversions
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
