//! FFmpeg filter-chain construction and execution for the editing engine.
//!
//! This module is the **single source of truth** for building ffmpeg argument
//! lists used by both the "Save Copy" (permanent duplicate) and "Render
//! Download" (on-demand stream) code paths.  The duplicated logic that
//! previously lived in `photos/copies.rs::render_video_copy` and
//! `photos/render.rs` is unified here.

use std::path::Path;

use tokio::process::Command;

use crate::error::AppError;
use crate::process::{run_with_timeout, FFMPEG_RENDER_TIMEOUT};

use super::models::CropMeta;

/// Build the `-vf` video filter string from edit metadata.
///
/// Returns an empty `Vec` when no video filters are needed.
///
/// Filter order: crop → rotate → brightness  (matches the client editor's
/// application order so server-rendered output is visually identical to the
/// client-side CSS preview).
pub fn build_video_filters(meta: &CropMeta) -> Vec<String> {
    let mut filters: Vec<String> = Vec::new();

    // Crop (fractional coordinates evaluated at runtime via ffmpeg expressions)
    if meta.has_crop() {
        let x = meta.x.unwrap_or(0.0);
        let y = meta.y.unwrap_or(0.0);
        let w = meta.width.unwrap_or(1.0);
        let h = meta.height.unwrap_or(1.0);
        filters.push(format!(
            "crop=iw*{w:.6}:ih*{h:.6}:iw*{x:.6}:ih*{y:.6}"
        ));
    }

    // Rotation via transpose (only cardinal angles)
    match meta.rotation_degrees() {
        90  => filters.push("transpose=1".into()),
        180 => {
            filters.push("vflip".into());
            filters.push("hflip".into());
        }
        270 => filters.push("transpose=2".into()),
        _   => {}
    }

    // Brightness via eq filter (-1.0 to 1.0; our scale is -100 to +100)
    if meta.has_brightness() {
        let b = meta.brightness.unwrap_or(0.0);
        filters.push(format!("eq=brightness={:.4}", b / 100.0));
    }

    filters
}

/// Build a complete `ffmpeg` argument list for rendering a video or audio file
/// with crop/rotation/brightness/trim edits.
///
/// The returned `Vec<String>` can be passed directly to `Command::args()`.
pub fn build_ffmpeg_args(
    source: &Path,
    dest: &Path,
    media_type: &str,
    meta: &CropMeta,
    ext: &str,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-i".into(),
        source.to_string_lossy().into_owned(),
    ];

    // Trim (output-side seeking for frame accuracy)
    if meta.has_trim_start() {
        args.push("-ss".into());
        args.push(format!("{:.6}", meta.trim_start.unwrap_or(0.0)));
    }
    if meta.has_trim_end() {
        args.push("-to".into());
        args.push(format!("{:.6}", meta.trim_end.unwrap_or(0.0)));
    }

    let needs_filter = media_type == "video" && meta.needs_video_filter();

    if needs_filter {
        let filters = build_video_filters(meta);
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
    } else if meta.has_trim() {
        // Trim-only (or audio): copy streams losslessly
        args.extend(["-c".into(), "copy".into()]);
    } else {
        // No meaningful edits — stream copy
        args.extend(["-c".into(), "copy".into()]);
    }

    // MP4: move moov atom to front for streaming playback
    if ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }

    args.push(dest.to_string_lossy().into_owned());
    args
}

/// Run ffmpeg with the given edit metadata, writing the result to `dest`.
///
/// Uses [`crate::process::run_with_timeout`] with the standard 120 s timeout,
/// `stdin(null)`, and `kill_on_drop(true)`.
pub async fn run_ffmpeg_render(
    source: &Path,
    dest: &Path,
    media_type: &str,
    meta: &CropMeta,
    ext: &str,
) -> Result<(), AppError> {
    let args = build_ffmpeg_args(source, dest, media_type, meta, ext);

    tracing::info!("[editing/ffmpeg] args: {:?}", args);

    let mut cmd = Command::new("ffmpeg");
    cmd.args(&args);
    let output = run_with_timeout(&mut cmd, FFMPEG_RENDER_TIMEOUT)
        .await
        .map_err(|e| AppError::Internal(format!("ffmpeg render: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("[editing/ffmpeg] ffmpeg failed:\n{}", stderr);
        let last_line = stderr.lines().last().unwrap_or("unknown error").to_string();
        return Err(AppError::Internal(format!(
            "ffmpeg render failed: {last_line}"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn meta(json: &str) -> CropMeta {
        CropMeta::from_json(json).unwrap()
    }

    #[test]
    fn no_edits_copies_stream() {
        let src = PathBuf::from("/tmp/in.mp4");
        let dst = PathBuf::from("/tmp/out.mp4");
        let m = meta(r#"{}"#);
        let args = build_ffmpeg_args(&src, &dst, "video", &m, "mp4");
        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"copy".to_string()));
        assert!(args.contains(&"+faststart".to_string()));
    }

    #[test]
    fn rotation_90_uses_transpose() {
        let m = meta(r#"{"rotate":90}"#);
        let filters = build_video_filters(&m);
        assert_eq!(filters, vec!["transpose=1"]);
    }

    #[test]
    fn rotation_180_uses_flip() {
        let m = meta(r#"{"rotate":180}"#);
        let filters = build_video_filters(&m);
        assert_eq!(filters, vec!["vflip", "hflip"]);
    }

    #[test]
    fn rotation_270_uses_transpose2() {
        let m = meta(r#"{"rotate":270}"#);
        let filters = build_video_filters(&m);
        assert_eq!(filters, vec!["transpose=2"]);
    }

    #[test]
    fn crop_produces_filter() {
        let m = meta(r#"{"x":0.1,"y":0.2,"width":0.6,"height":0.5}"#);
        let filters = build_video_filters(&m);
        assert_eq!(filters.len(), 1);
        assert!(filters[0].starts_with("crop="));
    }

    #[test]
    fn brightness_produces_eq_filter() {
        let m = meta(r#"{"brightness":50}"#);
        let filters = build_video_filters(&m);
        assert_eq!(filters.len(), 1);
        assert!(filters[0].starts_with("eq=brightness="));
    }

    #[test]
    fn combined_filters_ordered_crop_rotate_brightness() {
        let m = meta(r#"{"x":0.1,"y":0,"width":0.8,"height":1,"rotate":90,"brightness":20}"#);
        let filters = build_video_filters(&m);
        assert_eq!(filters.len(), 3);
        assert!(filters[0].starts_with("crop="));
        assert_eq!(filters[1], "transpose=1");
        assert!(filters[2].starts_with("eq=brightness="));
    }

    #[test]
    fn trim_only_copies_stream() {
        let src = PathBuf::from("/tmp/in.mp4");
        let dst = PathBuf::from("/tmp/out.mp4");
        let m = meta(r#"{"trimStart":2.0,"trimEnd":5.0}"#);
        let args = build_ffmpeg_args(&src, &dst, "video", &m, "mp4");
        assert!(args.contains(&"-ss".to_string()));
        assert!(args.contains(&"-to".to_string()));
        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"copy".to_string()));
        // No re-encode for trim-only
        assert!(!args.contains(&"libx264".to_string()));
    }

    #[test]
    fn video_filter_triggers_reencode() {
        let src = PathBuf::from("/tmp/in.mp4");
        let dst = PathBuf::from("/tmp/out.mp4");
        let m = meta(r#"{"rotate":90}"#);
        let args = build_ffmpeg_args(&src, &dst, "video", &m, "mp4");
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"aac".to_string()));
    }

    #[test]
    fn audio_never_uses_video_filter() {
        let src = PathBuf::from("/tmp/in.mp3");
        let dst = PathBuf::from("/tmp/out.mp3");
        let m = meta(r#"{"rotate":90,"brightness":50}"#);
        let args = build_ffmpeg_args(&src, &dst, "audio", &m, "mp3");
        // Audio: no -vf, no libx264
        assert!(!args.contains(&"-vf".to_string()));
        assert!(!args.contains(&"libx264".to_string()));
    }
}
