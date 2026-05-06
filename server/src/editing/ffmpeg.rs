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
use crate::transcode::HwAccelCapability;
use crate::transcode::gpu_probe::HwAccelType;

use super::models::CropMeta;

/// Encoder selection for the editing re-encode path.
///
/// Pairs the FFmpeg encoder + quality args with any required global
/// flags (e.g. `-vaapi_device`) and an optional filter-chain suffix
/// (e.g. `format=nv12,hwupload`) needed to feed software-decoded frames
/// into the hardware encoder.
struct EncoderPlan {
    /// Global args inserted BEFORE `-i` (e.g. `-vaapi_device /dev/dri/renderD128`).
    pre_input: Vec<String>,
    /// Encoder + quality args inserted AFTER the filter chain.
    encoder: Vec<String>,
    /// Optional filter chain suffix appended after user filters
    /// (with a leading comma added by the caller when filters exist).
    filter_suffix: Option<String>,
    /// Display label for logging.
    label: &'static str,
}

impl EncoderPlan {
    fn cpu() -> Self {
        Self {
            pre_input: Vec::new(),
            encoder: vec![
                "-c:v".into(), "libx264".into(),
                "-preset".into(), "fast".into(),
                "-crf".into(), "18".into(),
                "-c:a".into(), "aac".into(),
            ],
            filter_suffix: None,
            label: "libx264",
        }
    }

    fn from_hwaccel(h: &HwAccelCapability) -> Option<Self> {
        if !h.is_gpu() {
            return None;
        }
        Some(match h.accel_type {
            HwAccelType::Nvenc => Self {
                pre_input: Vec::new(),
                encoder: vec![
                    "-c:v".into(), "h264_nvenc".into(),
                    "-preset".into(), "p4".into(),
                    "-cq".into(), "18".into(),
                    "-c:a".into(), "aac".into(),
                ],
                filter_suffix: None,
                label: "h264_nvenc",
            },
            HwAccelType::Amf => Self {
                pre_input: Vec::new(),
                encoder: vec![
                    "-c:v".into(), "h264_amf".into(),
                    "-quality".into(), "balanced".into(),
                    "-rc".into(), "cqp".into(),
                    "-qp_i".into(), "18".into(),
                    "-qp_p".into(), "18".into(),
                    "-c:a".into(), "aac".into(),
                ],
                filter_suffix: None,
                label: "h264_amf",
            },
            HwAccelType::Vaapi => {
                let device = h.device.clone().unwrap_or_else(|| "/dev/dri/renderD128".into());
                Self {
                    pre_input: vec!["-vaapi_device".into(), device],
                    encoder: vec![
                        "-c:v".into(), "h264_vaapi".into(),
                        "-qp".into(), "18".into(),
                        "-c:a".into(), "aac".into(),
                    ],
                    filter_suffix: Some("format=nv12,hwupload".into()),
                    label: "h264_vaapi",
                }
            }
            HwAccelType::Qsv => Self {
                pre_input: vec![
                    "-init_hw_device".into(), "qsv=qsv:hw".into(),
                    "-filter_hw_device".into(), "qsv".into(),
                ],
                encoder: vec![
                    "-c:v".into(), "h264_qsv".into(),
                    "-preset".into(), "medium".into(),
                    "-global_quality".into(), "18".into(),
                    "-c:a".into(), "aac".into(),
                ],
                filter_suffix: Some("format=nv12,hwupload=extra_hw_frames=64".into()),
                label: "h264_qsv",
            },
            HwAccelType::Cpu => unreachable!("filtered above by is_gpu()"),
        })
    }
}

/// Build the `-vf` video filter string from edit metadata.
///
/// Returns an empty `Vec` when no video filters are needed.
///
/// Filter order: rotate → crop → brightness.  The crop fractional
/// coordinates are defined in the user's *displayed* (rotated) view, so we
/// must rotate the source first and then crop in that rotated coordinate
/// system.  Cropping before rotation produced misaligned output for
/// 90°/270° rotations and corrupted thumbnails when both transforms were
/// combined.  This matches `image_render::render_image()`.
pub fn build_video_filters(meta: &CropMeta) -> Vec<String> {
    let mut filters: Vec<String> = Vec::new();

    // Rotation via transpose (only cardinal angles) — applied FIRST so the
    // crop coordinates are interpreted in the rotated coordinate system the
    // user saw in the editor.
    match meta.rotation_degrees() {
        90  => filters.push("transpose=1".into()),
        180 => {
            filters.push("vflip".into());
            filters.push("hflip".into());
        }
        270 => filters.push("transpose=2".into()),
        _   => {}
    }

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
///
/// CPU-only variant — kept for backwards compatibility with unit tests and
/// callers that don't care about GPU acceleration. Internally delegates to
/// [`build_ffmpeg_args_with_plan`] with a CPU encoder plan.
#[allow(dead_code)] // Used by unit tests; production callers go through run_ffmpeg_render.
pub fn build_ffmpeg_args(
    source: &Path,
    dest: &Path,
    media_type: &str,
    meta: &CropMeta,
    ext: &str,
) -> Vec<String> {
    build_ffmpeg_args_with_plan(source, dest, media_type, meta, ext, &EncoderPlan::cpu())
}

/// Build ffmpeg arguments using the given encoder plan (CPU or GPU).
fn build_ffmpeg_args_with_plan(
    source: &Path,
    dest: &Path,
    media_type: &str,
    meta: &CropMeta,
    ext: &str,
    plan: &EncoderPlan,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::with_capacity(32);
    args.push("-y".into());

    // Hwaccel-specific global flags (e.g. -vaapi_device, -init_hw_device qsv)
    // must come before -i.
    args.extend(plan.pre_input.iter().cloned());

    args.push("-i".into());
    args.push(source.to_string_lossy().into_owned());

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
        let mut filters = build_video_filters(meta);
        if let Some(suffix) = &plan.filter_suffix {
            filters.push(suffix.clone());
        }
        if !filters.is_empty() {
            args.push("-vf".into());
            args.push(filters.join(","));
        }

        // Re-encode using the plan's encoder (libx264 / h264_nvenc / etc.)
        args.extend(plan.encoder.iter().cloned());
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
///
/// When a video re-encode is required and the system has GPU hardware
/// acceleration registered (NVENC / QSV / VAAPI / AMF), the GPU encoder
/// is tried first. On GPU failure (and `transcode.gpu_fallback_to_cpu = true`)
/// the function transparently retries with libx264.
pub async fn run_ffmpeg_render(
    source: &Path,
    dest: &Path,
    media_type: &str,
    meta: &CropMeta,
    ext: &str,
) -> Result<(), AppError> {
    let needs_reencode = media_type == "video" && meta.needs_video_filter();

    // Only video re-encodes benefit from GPU acceleration. Audio, trim-only,
    // and no-op renders use stream copy and don't touch the encoder.
    let gpu_plan = if needs_reencode {
        crate::conversion::active_hwaccel()
            .and_then(EncoderPlan::from_hwaccel)
    } else {
        None
    };

    if let Some(plan) = gpu_plan {
        let args = build_ffmpeg_args_with_plan(source, dest, media_type, meta, ext, &plan);
        tracing::info!(
            "[editing/ffmpeg] GPU render ({}): src={}, dst={}, media_type={}",
            plan.label, source.display(), dest.display(), media_type,
        );
        tracing::debug!("[editing/ffmpeg] GPU args: {:?}", args);

        let started = std::time::Instant::now();
        let mut cmd = Command::new("ffmpeg");
        cmd.args(&args);
        match run_with_timeout(&mut cmd, FFMPEG_RENDER_TIMEOUT).await {
            Ok(output) if output.status.success() => {
                tracing::info!(
                    encoder = plan.label,
                    elapsed_ms = started.elapsed().as_millis(),
                    "[editing/ffmpeg] GPU render succeeded"
                );
                return Ok(());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let last_line = stderr.lines().last().unwrap_or("unknown error");
                if !crate::conversion::cpu_fallback_enabled() {
                    tracing::error!(
                        encoder = plan.label,
                        "[editing/ffmpeg] GPU render failed and CPU fallback disabled: {last_line}"
                    );
                    return Err(AppError::Internal(format!(
                        "ffmpeg render failed ({}): {last_line}", plan.label
                    )));
                }
                tracing::warn!(
                    encoder = plan.label,
                    "[editing/ffmpeg] GPU render failed — retrying with libx264: {last_line}"
                );
                let _ = tokio::fs::remove_file(dest).await;
            }
            Err(e) => {
                if !crate::conversion::cpu_fallback_enabled() {
                    return Err(AppError::Internal(format!("ffmpeg render: {e}")));
                }
                tracing::warn!(
                    encoder = plan.label,
                    "[editing/ffmpeg] GPU render errored — retrying with libx264: {e}"
                );
                let _ = tokio::fs::remove_file(dest).await;
            }
        }
    }

    // CPU path (libx264) — used when no GPU is available, when the operation
    // doesn't require re-encoding, or as a fallback after GPU failure.
    let args = build_ffmpeg_args_with_plan(
        source, dest, media_type, meta, ext, &EncoderPlan::cpu(),
    );

    tracing::info!("[editing/ffmpeg] args: {:?}", args);
    tracing::info!(
        "[editing/ffmpeg] Rendering: src={}, dst={}, media_type={}, \
         has_crop={}, has_rotation={}, rotation={}°, has_brightness={}, has_trim={}",
        source.display(), dest.display(), media_type,
        meta.has_crop(), meta.has_rotation(), meta.rotation_degrees(),
        meta.has_brightness(), meta.has_trim(),
    );

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
    fn combined_filters_ordered_rotate_crop_brightness() {
        let m = meta(r#"{"x":0.1,"y":0,"width":0.8,"height":1,"rotate":90,"brightness":20}"#);
        let filters = build_video_filters(&m);
        assert_eq!(filters.len(), 3);
        assert_eq!(filters[0], "transpose=1");
        assert!(filters[1].starts_with("crop="));
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

    // ── DDT: GPU encoder plan selection ──────────────────────────────
    //
    // Verifies that `EncoderPlan::from_hwaccel` produces the correct
    // FFmpeg encoder, pre-input flags, and filter suffix for every
    // supported hardware-acceleration backend. CPU input must yield
    // `None` so the runner falls through to `EncoderPlan::cpu()`.

    fn cap(t: HwAccelType, encoder: &str, device: Option<&str>) -> HwAccelCapability {
        HwAccelCapability {
            accel_type: t,
            video_encoder: encoder.into(),
            device: device.map(|s| s.into()),
        }
    }

    #[test]
    fn encoder_plan_nvenc() {
        let p = EncoderPlan::from_hwaccel(&cap(HwAccelType::Nvenc, "h264_nvenc", None)).unwrap();
        assert_eq!(p.label, "h264_nvenc");
        assert!(p.pre_input.is_empty());
        assert!(p.encoder.iter().any(|s| s == "h264_nvenc"));
        assert!(p.filter_suffix.is_none());
    }

    #[test]
    fn encoder_plan_amf() {
        let p = EncoderPlan::from_hwaccel(&cap(HwAccelType::Amf, "h264_amf", None)).unwrap();
        assert_eq!(p.label, "h264_amf");
        assert!(p.pre_input.is_empty());
        assert!(p.encoder.iter().any(|s| s == "h264_amf"));
        assert!(p.filter_suffix.is_none());
    }

    #[test]
    fn encoder_plan_vaapi_uses_device_and_hwupload() {
        let p = EncoderPlan::from_hwaccel(&cap(
            HwAccelType::Vaapi, "h264_vaapi", Some("/dev/dri/renderD128"),
        ))
        .unwrap();
        assert_eq!(p.label, "h264_vaapi");
        assert_eq!(
            p.pre_input,
            vec!["-vaapi_device".to_string(), "/dev/dri/renderD128".to_string()]
        );
        assert!(p.encoder.iter().any(|s| s == "h264_vaapi"));
        assert_eq!(p.filter_suffix.as_deref(), Some("format=nv12,hwupload"));
    }

    #[test]
    fn encoder_plan_vaapi_default_device_when_missing() {
        let p = EncoderPlan::from_hwaccel(&cap(HwAccelType::Vaapi, "h264_vaapi", None)).unwrap();
        assert_eq!(p.pre_input[1], "/dev/dri/renderD128");
    }

    #[test]
    fn encoder_plan_qsv() {
        let p = EncoderPlan::from_hwaccel(&cap(HwAccelType::Qsv, "h264_qsv", None)).unwrap();
        assert_eq!(p.label, "h264_qsv");
        assert!(p.pre_input.iter().any(|s| s == "-init_hw_device"));
        assert!(p.encoder.iter().any(|s| s == "h264_qsv"));
        assert!(p.filter_suffix.as_deref().unwrap().contains("hwupload"));
    }

    #[test]
    fn encoder_plan_cpu_returns_none() {
        assert!(EncoderPlan::from_hwaccel(&cap(HwAccelType::Cpu, "libx264", None)).is_none());
    }

    #[test]
    fn gpu_plan_appends_filter_suffix_after_user_filters() {
        let src = PathBuf::from("/tmp/in.mp4");
        let dst = PathBuf::from("/tmp/out.mp4");
        let m = meta(r#"{"rotate":90}"#);
        let plan = EncoderPlan::from_hwaccel(&cap(
            HwAccelType::Vaapi, "h264_vaapi", Some("/dev/dri/renderD128"),
        ))
        .unwrap();
        let args = build_ffmpeg_args_with_plan(&src, &dst, "video", &m, "mp4", &plan);
        // -vaapi_device must appear BEFORE -i for FFmpeg to honour it.
        let dev_pos = args.iter().position(|s| s == "-vaapi_device").unwrap();
        let input_pos = args.iter().position(|s| s == "-i").unwrap();
        assert!(dev_pos < input_pos, "global hwaccel flag must precede -i");

        // Filter suffix must be appended after user filters, comma-joined.
        let vf_idx = args.iter().position(|s| s == "-vf").unwrap();
        let vf = &args[vf_idx + 1];
        assert!(vf.starts_with("transpose=1"), "user filter first: {vf}");
        assert!(vf.ends_with("format=nv12,hwupload"), "hw suffix last: {vf}");

        // Encoder is hardware, NOT libx264.
        assert!(args.iter().any(|s| s == "h264_vaapi"));
        assert!(!args.iter().any(|s| s == "libx264"));
    }

    #[test]
    fn gpu_plan_skipped_when_no_reencode() {
        // Trim-only operations stream-copy regardless of encoder plan.
        let src = PathBuf::from("/tmp/in.mp4");
        let dst = PathBuf::from("/tmp/out.mp4");
        let m = meta(r#"{"trimStart":1.0,"trimEnd":3.0}"#);
        let plan = EncoderPlan::from_hwaccel(&cap(HwAccelType::Nvenc, "h264_nvenc", None)).unwrap();
        let args = build_ffmpeg_args_with_plan(&src, &dst, "video", &m, "mp4", &plan);
        assert!(args.iter().any(|s| s == "copy"));
        assert!(!args.iter().any(|s| s == "h264_nvenc"));
    }
}
