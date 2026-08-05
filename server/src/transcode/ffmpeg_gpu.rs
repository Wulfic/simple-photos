//! GPU-accelerated FFmpeg command-line builder.
//!
//! Constructs FFmpeg arguments for video transcoding using the detected
//! hardware acceleration backend.  Each backend uses its optimal flags
//! (NVENC, QSV, VAAPI, AMF) with automatic CPU fallback.

use super::gpu_probe::{HwAccelCapability, HwAccelType};

/// Target dimensions for a ladder rung (#49), in pixels.
///
/// `None` means "encode at source resolution" — the behaviour every caller had
/// before the ladder existed, and still the behaviour for the ~81% of the live
/// library that sits at or below the 1080p tier.
///
/// The dimensions are computed by [`super::ladder::rung_dimensions`] from the
/// probe's geometry, **not** by an ffmpeg scale expression. That is deliberate:
/// expressing "scale the short edge to 1080, preserving orientation" in ffmpeg
/// expression syntax is both unreadable and untestable, and getting it wrong
/// downscales portrait 1080p video to 608x1080. Doing the arithmetic in Rust
/// makes it a unit test instead of a device test.
pub type RungSize = Option<(i64, i64)>;

/// Scale filter for the software filter graph, or `None` at source resolution.
///
/// At source resolution the existing `trunc(...)` form is preserved verbatim —
/// it applies the sample aspect ratio and forces even dimensions, and it has
/// been the shipping behaviour for every video in the library.
///
/// For a rung the dimensions are already exact and already even, so the filter
/// is a plain `scale=W:H`. `setsar=1` still follows, because a source with a
/// non-square SAR would otherwise carry it into the rendition and display at
/// the wrong shape.
pub(crate) fn software_scale(rung: RungSize) -> String {
    match rung {
        Some((w, h)) => format!("scale={w}:{h},setsar=1"),
        None => "scale=trunc(iw*sar/2)*2:trunc(ih/2)*2,setsar=1".to_string(),
    }
}

/// Build FFmpeg arguments for video → MP4 transcoding using the given
/// hardware acceleration backend.
///
/// All GPU paths include `-movflags +faststart` for web streaming and
/// AAC audio at 192 kbps.  Quality target is roughly equivalent to
/// libx264 CRF 20 across all backends.
///
/// `rung` selects a resolution-ladder output (#49); see [`RungSize`].
pub fn build_video_transcode_args(
    input: &str,
    output: &str,
    hwaccel: &HwAccelCapability,
    rung: RungSize,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::with_capacity(24);

    match hwaccel.accel_type {
        HwAccelType::Nvenc => {
            // Decode on the CPU and encode on the GPU (NVENC). We deliberately
            // do NOT use `-hwaccel cuda -hwaccel_output_format cuda`: that full
            // GPU pipeline keeps frames in VRAM and *requires* NVDEC to decode
            // the source, so it hard-fails on any codec NVDEC can't handle
            // (older MPEG-4/DivX, VP9, some 10-bit HEVC, etc.) instead of
            // falling back — which is exactly the "GPU conversion is failing"
            // report even though CUDA (AI) works fine. CPU decode + NVENC encode
            // keeps the expensive H.264 encode on the GPU while accepting any
            // input ffmpeg can read.
            //
            // The scale filter forces even dimensions (NVENC rejects odd
            // width/height) and `format=yuv420p` normalises exotic pixel formats
            // (yuv444/10-bit) that h264_nvenc would otherwise refuse.
            args.extend([
                "-y".into(),
                "-i".into(),
                input.into(),
                "-vf".into(),
                format!("{},format=yuv420p", software_scale(rung)),
                "-c:v".into(),
                "h264_nvenc".into(),
                "-preset".into(),
                "p4".into(),
                "-cq".into(),
                "20".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-movflags".into(),
                "+faststart".into(),
                output.into(),
            ]);
        }
        HwAccelType::Qsv => {
            args.extend([
                "-y".into(),
                "-hwaccel".into(),
                "qsv".into(),
                "-i".into(),
                input.into(),
            ]);
            // Only added for a ladder rung. At source resolution this branch
            // has never carried a filter graph, and adding an inert one would
            // change the shipping command line for every QSV transcode in
            // order to fix a case that does not apply to it.
            if let Some((w, h)) = rung {
                args.push("-vf".into());
                args.push(format!("scale_qsv=w={w}:h={h}"));
            }
            args.extend([
                "-c:v".into(),
                "h264_qsv".into(),
                "-preset".into(),
                "medium".into(),
                "-global_quality".into(),
                "20".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-movflags".into(),
                "+faststart".into(),
                output.into(),
            ]);
        }
        HwAccelType::Vaapi => {
            let device = hwaccel.device.as_deref().unwrap_or("/dev/dri/renderD128");
            args.extend([
                "-y".into(),
                "-hwaccel".into(),
                "vaapi".into(),
                "-hwaccel_device".into(),
                device.into(),
                "-hwaccel_output_format".into(),
                "vaapi".into(),
                "-i".into(),
                input.into(),
                "-vf".into(),
                // VAAPI keeps frames in GPU memory (`-hwaccel_output_format
                // vaapi`), so the scale must be the VAAPI one — a software
                // `scale` here would fail on hardware frames.
                match rung {
                    Some((w, h)) => format!("scale_vaapi=w={w}:h={h}:format=nv12"),
                    None => "scale_vaapi=format=nv12".to_string(),
                },
                "-c:v".into(),
                "h264_vaapi".into(),
                "-qp".into(),
                "20".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-movflags".into(),
                "+faststart".into(),
                output.into(),
            ]);
        }
        HwAccelType::Amf => {
            args.extend([
                "-y".into(),
                "-hwaccel".into(),
                "d3d11va".into(),
                "-i".into(),
                input.into(),
            ]);
            // `-hwaccel d3d11va` without `-hwaccel_output_format d3d11` hands
            // decoded frames back in system memory, so the ordinary software
            // scaler applies. Same reasoning as QSV for only adding it on a rung.
            if let Some((w, h)) = rung {
                args.push("-vf".into());
                args.push(format!("scale={w}:{h},setsar=1"));
            }
            args.extend([
                "-c:v".into(),
                "h264_amf".into(),
                "-quality".into(),
                "balanced".into(),
                "-rc".into(),
                "cqp".into(),
                "-qp_i".into(),
                "20".into(),
                "-qp_p".into(),
                "20".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-movflags".into(),
                "+faststart".into(),
                output.into(),
            ]);
        }
        HwAccelType::Cpu => {
            args.extend([
                "-y".into(),
                "-i".into(),
                input.into(),
                "-vf".into(),
                software_scale(rung),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "medium".into(),
                "-crf".into(),
                "20".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-movflags".into(),
                "+faststart".into(),
                output.into(),
            ]);
        }
    }

    args
}

/// Build the CPU (libx264) command line used both as the standalone software
/// path and as the fallback after a failed GPU encode.
///
/// Extracted from `conversion::convert_video`, where it was an inline
/// `cmd.args([...])` holding its own hardcoded copy of the scale filter. That
/// duplication was harmless while source resolution was the only target, but
/// under the ladder it is a silent correctness bug: a rung encode that fails
/// over to CPU would emit a full-resolution file which is then recorded as the
/// 1080p rendition, so the user picks "1080p" and downloads the 4K bytes.
///
/// Being a function rather than inline args is what makes that testable at all.
///
/// `video_threads` bounds the encoder when several encodes run in parallel
/// (bulk import); `None` means ffmpeg auto (all cores) for a lone encode.
pub fn build_cpu_fallback_args(
    input: &str,
    output: &str,
    video_threads: Option<usize>,
    rung: RungSize,
) -> Vec<String> {
    vec![
        "-y".into(),
        "-i".into(),
        input.into(),
        "-threads".into(),
        video_threads.map(|n| n.max(1)).unwrap_or(0).to_string(),
        "-vf".into(),
        software_scale(rung),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "medium".into(),
        "-crf".into(),
        "20".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "192k".into(),
        "-movflags".into(),
        "+faststart".into(),
        output.into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcode::gpu_probe::HwAccelType;

    const ALL_BACKENDS: &[HwAccelType] = &[
        HwAccelType::Nvenc,
        HwAccelType::Qsv,
        HwAccelType::Vaapi,
        HwAccelType::Amf,
        HwAccelType::Cpu,
    ];

    fn cap(accel_type: HwAccelType) -> HwAccelCapability {
        HwAccelCapability {
            accel_type,
            device: None,
            video_encoder: "test".into(),
        }
    }

    /// Joined filter graph for a backend, or `None` if it has no `-vf`.
    fn filter_graph(accel_type: HwAccelType, rung: RungSize) -> Option<String> {
        let args = build_video_transcode_args("in.mp4", "out.mp4", &cap(accel_type), rung);
        args.iter()
            .position(|a| a == "-vf")
            .map(|i| args[i + 1].clone())
    }

    /// The safety property that matters most: with no rung requested, every
    /// backend must produce the exact command line it produced before the
    /// ladder existed. 742 videos convert through this path; a stray filter
    /// change here is a library-wide regression to chase.
    #[test]
    fn source_resolution_command_lines_are_unchanged() {
        assert_eq!(
            filter_graph(HwAccelType::Nvenc, None).as_deref(),
            Some("scale=trunc(iw*sar/2)*2:trunc(ih/2)*2,setsar=1,format=yuv420p")
        );
        assert_eq!(
            filter_graph(HwAccelType::Cpu, None).as_deref(),
            Some("scale=trunc(iw*sar/2)*2:trunc(ih/2)*2,setsar=1")
        );
        assert_eq!(
            filter_graph(HwAccelType::Vaapi, None).as_deref(),
            Some("scale_vaapi=format=nv12")
        );
        // QSV and AMF have never carried a filter graph at source resolution,
        // and must not gain an inert one.
        assert_eq!(filter_graph(HwAccelType::Qsv, None), None);
        assert_eq!(filter_graph(HwAccelType::Amf, None), None);
    }

    /// Every backend must actually apply the rung. A backend that silently
    /// ignores it produces a full-resolution file recorded as a 1080p
    /// rendition — the user picks "1080p" and downloads the 4K bytes.
    #[test]
    fn every_backend_applies_the_requested_rung() {
        for &accel in ALL_BACKENDS {
            let graph = filter_graph(accel, Some((1920, 1080)))
                .unwrap_or_else(|| panic!("{accel:?} produced no filter graph for a rung"));
            assert!(
                graph.contains("1920") && graph.contains("1080"),
                "{accel:?} dropped the rung dimensions: {graph}"
            );
        }
    }

    /// VAAPI keeps frames in GPU memory, so it needs `scale_vaapi`; a software
    /// `scale` fails outright on hardware frames. QSV likewise.
    #[test]
    fn hardware_frame_backends_use_their_own_scaler() {
        let vaapi = filter_graph(HwAccelType::Vaapi, Some((1920, 1080))).unwrap();
        assert_eq!(vaapi, "scale_vaapi=w=1920:h=1080:format=nv12");
        assert!(
            !vaapi.starts_with("scale="),
            "a software scale filter cannot consume VAAPI hardware frames"
        );

        let qsv = filter_graph(HwAccelType::Qsv, Some((1920, 1080))).unwrap();
        assert_eq!(qsv, "scale_qsv=w=1920:h=1080");
    }

    /// A rung's dimensions come from `ladder::rung_dimensions`, which has
    /// already applied SAR-free even-dimension arithmetic. Re-deriving them
    /// with `trunc(iw*sar/...)` would scale the *already scaled* frame.
    #[test]
    fn a_rung_replaces_the_source_scale_rather_than_composing_with_it() {
        let graph = filter_graph(HwAccelType::Cpu, Some((1440, 1080))).unwrap();
        assert_eq!(graph, "scale=1440:1080,setsar=1");
        assert!(
            !graph.contains("trunc"),
            "the source-resolution expression must not survive alongside a rung"
        );
    }

    /// Portrait rungs are the case the whole short-edge rule exists for. The
    /// builder must pass them through untouched rather than re-orienting.
    #[test]
    fn portrait_rung_dimensions_are_passed_through_verbatim() {
        let graph = filter_graph(HwAccelType::Cpu, Some((1080, 1920))).unwrap();
        assert_eq!(
            graph, "scale=1080:1920,setsar=1",
            "width and height must not be swapped on their way to ffmpeg"
        );
    }

    /// `setsar=1` must survive on the rung path: a source with non-square
    /// sample aspect would otherwise carry it into the rendition and display
    /// at the wrong shape.
    #[test]
    fn rungs_still_normalise_sample_aspect_ratio() {
        for accel in [HwAccelType::Nvenc, HwAccelType::Cpu, HwAccelType::Amf] {
            let graph = filter_graph(accel, Some((1920, 1080))).unwrap();
            assert!(graph.contains("setsar=1"), "{accel:?}: {graph}");
        }
    }

    /// NVENC additionally normalises exotic pixel formats; losing that on the
    /// rung path would make 10-bit sources fail to encode.
    #[test]
    fn nvenc_keeps_its_pixel_format_normalisation_on_a_rung() {
        let graph = filter_graph(HwAccelType::Nvenc, Some((1920, 1080))).unwrap();
        assert_eq!(graph, "scale=1920:1080,setsar=1,format=yuv420p");
    }

    fn cpu_fallback_graph(rung: RungSize) -> String {
        let args = build_cpu_fallback_args("in.mp4", "out.mp4", Some(4), rung);
        let i = args.iter().position(|a| a == "-vf").expect("no -vf");
        args[i + 1].clone()
    }

    /// The trap this extraction exists for. The GPU path is the one that
    /// *asks* for a rung, but the CPU fallback is what actually runs whenever
    /// the hardware encode fails — which for a 4K HEVC source is common, since
    /// that is exactly the input NVDEC/QSV struggle with.
    ///
    /// If this regresses, nothing fails loudly: a full-resolution file is
    /// produced, recorded as the 1080p rendition, and served to anyone who
    /// picks "1080p". The bug is only visible as "the quality picker does
    /// nothing", which is unattributable in a bug report.
    #[test]
    fn the_cpu_fallback_honours_the_rung_the_gpu_path_was_given() {
        assert_eq!(
            cpu_fallback_graph(Some((1920, 1080))),
            "scale=1920:1080,setsar=1"
        );
        assert_eq!(
            cpu_fallback_graph(Some((1080, 1920))),
            "scale=1080:1920,setsar=1"
        );
    }

    /// ...and at source resolution it must still emit exactly what it always
    /// did, since every non-ladder conversion in the library uses this path.
    #[test]
    fn the_cpu_fallback_is_unchanged_at_source_resolution() {
        assert_eq!(
            cpu_fallback_graph(None),
            "scale=trunc(iw*sar/2)*2:trunc(ih/2)*2,setsar=1"
        );
    }

    /// The GPU path and its fallback must agree on the target, or the fallback
    /// silently changes what was asked for. Comparing the two graphs directly
    /// is what pins that, rather than two independent literal assertions that
    /// can drift apart.
    #[test]
    fn the_cpu_fallback_and_the_cpu_backend_target_the_same_size() {
        for rung in [None, Some((1920, 1080)), Some((1440, 1080))] {
            assert_eq!(
                cpu_fallback_graph(rung),
                filter_graph(HwAccelType::Cpu, rung).unwrap(),
                "fallback and software backend disagree for {rung:?}"
            );
        }
    }

    /// Thread bounding is load-bearing for bulk import (video_lane ×
    /// video_threads must not oversubscribe the box) and is easy to drop while
    /// refactoring the args into a builder.
    #[test]
    fn thread_bounding_survives_the_extraction() {
        let bounded = build_cpu_fallback_args("in.mp4", "out.mp4", Some(3), None);
        let i = bounded.iter().position(|a| a == "-threads").unwrap();
        assert_eq!(bounded[i + 1], "3");

        // None means ffmpeg auto — spelled `0`, not omitted.
        let auto = build_cpu_fallback_args("in.mp4", "out.mp4", None, None);
        let j = auto.iter().position(|a| a == "-threads").unwrap();
        assert_eq!(auto[j + 1], "0");

        // A caller passing 0 must not accidentally mean "auto".
        let clamped = build_cpu_fallback_args("in.mp4", "out.mp4", Some(0), None);
        let k = clamped.iter().position(|a| a == "-threads").unwrap();
        assert_eq!(clamped[k + 1], "1");
    }
}
