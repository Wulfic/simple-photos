//! GPU-accelerated FFmpeg command-line builder.
//!
//! Constructs FFmpeg arguments for video transcoding using the detected
//! hardware acceleration backend.  Each backend uses its optimal flags
//! (NVENC, QSV, VAAPI, AMF) with automatic CPU fallback.

use super::gpu_probe::{HwAccelCapability, HwAccelType};

/// Build FFmpeg arguments for video → MP4 transcoding using the given
/// hardware acceleration backend.
///
/// All GPU paths include `-movflags +faststart` for web streaming and
/// AAC audio at 192 kbps.  Quality target is roughly equivalent to
/// libx264 CRF 20 across all backends.
pub fn build_video_transcode_args(
    input: &str,
    output: &str,
    hwaccel: &HwAccelCapability,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::with_capacity(24);

    match hwaccel.accel_type {
        HwAccelType::Nvenc => {
            args.extend([
                "-y".into(),
                "-hwaccel".into(), "cuda".into(),
                "-hwaccel_output_format".into(), "cuda".into(),
                "-i".into(), input.into(),
                "-c:v".into(), "h264_nvenc".into(),
                "-preset".into(), "p4".into(),
                "-cq".into(), "20".into(),
                "-c:a".into(), "aac".into(),
                "-b:a".into(), "192k".into(),
                "-movflags".into(), "+faststart".into(),
                output.into(),
            ]);
        }
        HwAccelType::Qsv => {
            args.extend([
                "-y".into(),
                "-hwaccel".into(), "qsv".into(),
                "-i".into(), input.into(),
                "-c:v".into(), "h264_qsv".into(),
                "-preset".into(), "medium".into(),
                "-global_quality".into(), "20".into(),
                "-c:a".into(), "aac".into(),
                "-b:a".into(), "192k".into(),
                "-movflags".into(), "+faststart".into(),
                output.into(),
            ]);
        }
        HwAccelType::Vaapi => {
            let device = hwaccel.device.as_deref().unwrap_or("/dev/dri/renderD128");
            args.extend([
                "-y".into(),
                "-hwaccel".into(), "vaapi".into(),
                "-hwaccel_device".into(), device.into(),
                "-hwaccel_output_format".into(), "vaapi".into(),
                "-i".into(), input.into(),
                "-vf".into(), "scale_vaapi=format=nv12".into(),
                "-c:v".into(), "h264_vaapi".into(),
                "-qp".into(), "20".into(),
                "-c:a".into(), "aac".into(),
                "-b:a".into(), "192k".into(),
                "-movflags".into(), "+faststart".into(),
                output.into(),
            ]);
        }
        HwAccelType::Amf => {
            args.extend([
                "-y".into(),
                "-hwaccel".into(), "d3d11va".into(),
                "-i".into(), input.into(),
                "-c:v".into(), "h264_amf".into(),
                "-quality".into(), "balanced".into(),
                "-rc".into(), "cqp".into(),
                "-qp_i".into(), "20".into(),
                "-qp_p".into(), "20".into(),
                "-c:a".into(), "aac".into(),
                "-b:a".into(), "192k".into(),
                "-movflags".into(), "+faststart".into(),
                output.into(),
            ]);
        }
        HwAccelType::Cpu => {
            args.extend([
                "-y".into(),
                "-i".into(), input.into(),
                "-vf".into(), "scale=trunc(iw*sar/2)*2:trunc(ih/2)*2,setsar=1".into(),
                "-c:v".into(), "libx264".into(),
                "-preset".into(), "medium".into(),
                "-crf".into(), "20".into(),
                "-c:a".into(), "aac".into(),
                "-b:a".into(), "192k".into(),
                "-movflags".into(), "+faststart".into(),
                output.into(),
            ]);
        }
    }

    args
}
