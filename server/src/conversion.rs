//! Media format conversion pipeline — converts non-native formats to
//! browser-compatible equivalents using FFmpeg.
//!
//! Conversion targets:
//! - Images (HEIC, TIFF, RAW, etc.) → JPEG
//! - Videos (MKV, AVI, MOV, etc.)   → MP4 (H.264/AAC)
//! - Audio  (WMA, AIFF, M4A, etc.)  → MP3
//!
//! Quality is tuned for visual/audible fidelity while keeping file sizes
//! manageable.  FFmpeg must be installed on the host system.

use std::path::Path;
use std::sync::atomic::{AtomicI64, AtomicBool, Ordering};
use std::sync::OnceLock;

use axum::Json;
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::transcode::{HwAccelCapability, HwAccelType};

// ── GPU acceleration config (set once at startup) ────────────────────────────

/// Cached GPU hardware acceleration capability, set by `init_gpu_config()`.
static GPU_CONFIG: OnceLock<GpuConversionConfig> = OnceLock::new();

struct GpuConversionConfig {
    hwaccel: HwAccelCapability,
    fallback_to_cpu: bool,
}

/// Initialize GPU conversion config. Called once from main.rs at startup.
pub fn init_gpu_config(hwaccel: HwAccelCapability, fallback_to_cpu: bool) {
    let _ = GPU_CONFIG.set(GpuConversionConfig { hwaccel, fallback_to_cpu });
}

/// Get the current GPU config, or None if not initialized.
fn gpu_config() -> Option<&'static GpuConversionConfig> {
    GPU_CONFIG.get()
}

// ── Conversion progress tracking ─────────────────────────────────────────────

/// Global conversion progress counters, polled by the frontend banner.
static CONV_ACTIVE: AtomicBool = AtomicBool::new(false);
static CONV_TOTAL: AtomicI64 = AtomicI64::new(0);
static CONV_DONE: AtomicI64 = AtomicI64::new(0);

/// Start a new conversion batch (resets counters).
pub fn progress_start(total: i64) {
    CONV_DONE.store(0, Ordering::Relaxed);
    CONV_TOTAL.store(total, Ordering::Relaxed);
    CONV_ACTIVE.store(true, Ordering::Relaxed);
}

/// Increment the done counter by 1.
pub fn progress_tick() {
    CONV_DONE.fetch_add(1, Ordering::Relaxed);
}

/// Signal that the conversion batch is complete.
pub fn progress_finish() {
    CONV_ACTIVE.store(false, Ordering::Relaxed);
}

/// Read the current conversion progress snapshot.
/// `done` is clamped to `total` as a safety net against races.
pub fn progress_snapshot() -> (bool, i64, i64) {
    let active = CONV_ACTIVE.load(Ordering::Relaxed);
    let total = CONV_TOTAL.load(Ordering::Relaxed);
    let done = CONV_DONE.load(Ordering::Relaxed).min(total);
    (active, total, done)
}

// ── Media categories ─────────────────────────────────────────────────────────

/// Broad media category used to select conversion parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCategory {
    Image,
    Video,
    Audio,
}

/// Describes the target format for a conversion.
#[derive(Debug, Clone)]
pub struct ConversionTarget {
    pub extension: &'static str,
    pub mime_type: &'static str,
    pub category: MediaCategory,
}

// ── Extension → target mapping ───────────────────────────────────────────────

/// Determine the conversion target for a file based on its extension.
/// Returns `None` if the file is already a native format or is not a
/// recognised convertible format.
pub fn conversion_target(filename: &str) -> Option<ConversionTarget> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        // ── Images → JPEG ────────────────────────────────────────────
        "heic" | "heif"                                     // Apple
        | "tiff" | "tif"                                    // Tagged Image
        | "cr2" | "cr3"                                     // Canon RAW
        | "dng"                                             // Adobe DNG
        | "nef"                                             // Nikon RAW
        | "arw"                                             // Sony RAW
        | "orf"                                             // Olympus RAW
        | "rw2"                                             // Panasonic RAW
        | "pef"                                             // Pentax RAW
        | "sr2" | "srf"                                     // Sony RAW (older)
        | "raf"                                             // Fujifilm RAW
        | "raw"                                             // Generic RAW
        | "hdr"                                             // Radiance HDR
        | "exr"                                             // OpenEXR
        | "psd"                                             // Photoshop
        | "tga"                                             // Targa
        | "pcx"                                             // PC Paintbrush
        | "ppm" | "pgm" | "pbm" | "pnm"                    // Netpbm
        | "xbm" | "xpm"                                    // X11 bitmap
        | "jp2" | "j2k" | "jpx"                            // JPEG 2000
        | "jxl"                                             // JPEG XL
        | "jfif" | "jpe"                                    // JPEG variants
        | "cur"                                             // Windows cursor
        => Some(ConversionTarget {
            extension: "jpg",
            mime_type: "image/jpeg",
            category: MediaCategory::Image,
        }),

        // ── Videos → MP4 (H.264 + AAC) ──────────────────────────────
        "mkv"                                               // Matroska
        | "avi"                                             // AVI
        | "wmv"                                             // Windows Media
        | "mov"                                             // QuickTime
        | "m4v"                                             // iTunes Video
        | "flv" | "f4v"                                     // Flash Video
        | "3gp" | "3g2"                                     // 3GPP
        | "mpg" | "mpeg"                                    // MPEG-1/2
        | "ts" | "mts" | "m2ts"                             // MPEG transport
        | "vob"                                             // DVD
        | "asf"                                             // ASF container
        | "rm" | "rmvb"                                     // RealMedia
        | "divx"                                            // DivX
        | "ogv"                                             // Ogg Video
        | "mxf"                                             // Material Exchange
        | "dv"                                              // Digital Video
        | "hevc" | "h264" | "h265"                          // Raw codec streams
        => Some(ConversionTarget {
            extension: "mp4",
            mime_type: "video/mp4",
            category: MediaCategory::Video,
        }),

        // ── Audio → MP3 ─────────────────────────────────────────────
        "wma"                                               // Windows Media Audio
        | "aiff" | "aif"                                    // Apple AIFF
        | "m4a"                                             // AAC container
        | "aac"                                             // Raw AAC
        | "wv"                                              // WavPack
        | "ape"                                             // Monkey's Audio
        | "opus"                                            // Opus
        | "ra" | "ram"                                      // RealAudio
        | "amr"                                             // Adaptive Multi-Rate
        | "ac3"                                             // Dolby AC3
        | "dts"                                             // DTS audio
        | "tta"                                             // True Audio
        | "mka"                                             // Matroska audio
        | "au" | "snd"                                      // Sun/NeXT audio
        | "caf"                                             // Core Audio
        | "spx"                                             // Speex
        | "dsf" | "dff"                                     // DSD audio
        => Some(ConversionTarget {
            extension: "mp3",
            mime_type: "audio/mpeg",
            category: MediaCategory::Audio,
        }),

        _ => None,
    }
}

/// Check whether a file can be converted to a browser-native format.
pub fn is_convertible(filename: &str) -> bool {
    conversion_target(filename).is_some()
}

/// Media-type string for the database (`photo`, `video`, `audio`, `gif`).
pub fn media_type_str(cat: MediaCategory) -> &'static str {
    match cat {
        MediaCategory::Image => "photo",
        MediaCategory::Video => "video",
        MediaCategory::Audio => "audio",
    }
}

// ── FFmpeg conversion ────────────────────────────────────────────────────────

/// Convert a media file to its browser-native target format.
///
/// Uses quality-tuned FFmpeg parameters:
/// - **Images** → JPEG at `-q:v 2` (near-lossless, ~92% quality)
/// - **Videos** → MP4 H.264 at `-crf 20 -preset medium`, AAC 192 kbps
/// - **Audio**  → MP3 at 192 kbps via libmp3lame
///
/// Falls back to ImageMagick for images if FFmpeg fails (e.g. RAW formats
/// that require specific decoders).
///
/// For video conversions, uses GPU-accelerated encoding when available
/// (configured via `init_gpu_config()` at startup).
pub async fn convert_file(
    input: &Path,
    output: &Path,
    target: &ConversionTarget,
) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Create output directory: {}", e))?;
    }

    let input_str = input.to_str().ok_or("Invalid input path encoding")?;
    let output_str = output.to_str().ok_or("Invalid output path encoding")?;

    let success = match target.category {
        MediaCategory::Image => convert_image(input_str, output_str).await,
        MediaCategory::Video => {
            let gpu = gpu_config();
            convert_video(
                input_str,
                output_str,
                gpu.map(|g| &g.hwaccel),
                gpu.map(|g| g.fallback_to_cpu).unwrap_or(true),
            ).await
        }
        MediaCategory::Audio => convert_audio(input_str, output_str).await,
    };

    if !success {
        let _ = tokio::fs::remove_file(output).await;
        return Err(format!(
            "Conversion failed for '{}' → .{}",
            input.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            target.extension,
        ));
    }

    // Verify the output file exists and is non-empty.
    match tokio::fs::metadata(output).await {
        Ok(m) if m.len() > 0 => Ok(()),
        Ok(_) => {
            let _ = tokio::fs::remove_file(output).await;
            Err("Conversion produced an empty file".into())
        }
        Err(e) => Err(format!("Output file missing after conversion: {}", e)),
    }
}

// ── Format-specific converters ───────────────────────────────────────────────

/// Image → JPEG.  Tries FFmpeg first, falls back to ImageMagick.
async fn convert_image(input: &str, output: &str) -> bool {
    // FFmpeg: high-quality JPEG output (-q:v 2 ≈ 92% quality).
    let mut cmd = tokio::process::Command::new("nice");
    cmd.args(["-n", "19", "ffmpeg", "-y", "-i", input, "-q:v", "2", output])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let ffmpeg = crate::process::status_with_timeout(
        &mut cmd,
        std::time::Duration::from_secs(600),
    ).await;

    if matches!(ffmpeg, Ok(s) if s.success()) {
        return true;
    }

    // Fallback: ImageMagick `convert` (handles RAW, PSD, SVG, etc.)
    let mut cmd = tokio::process::Command::new("convert");
    cmd.args([
            &format!("{}[0]", input), // [0] = first frame/page
            "-quality",
            "92",
            "-auto-orient",
            output,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let magick = crate::process::status_with_timeout(
        &mut cmd,
        std::time::Duration::from_secs(600),
    ).await;

    matches!(magick, Ok(s) if s.success())
}

/// Video → MP4 (H.264 + AAC).  Quality-tuned for clarity at reasonable sizes.
/// When a GPU `hwaccel` capability is provided, uses hardware-accelerated
/// encoding.  Falls back to CPU (libx264) if the GPU transcode fails and
/// `fallback_to_cpu` is true.
async fn convert_video(
    input: &str,
    output: &str,
    hwaccel: Option<&HwAccelCapability>,
    fallback_to_cpu: bool,
) -> bool {
    // Try GPU path first if available
    if let Some(hw) = hwaccel {
        if hw.is_gpu() {
            let args = crate::transcode::ffmpeg_gpu::build_video_transcode_args(input, output, hw);
            let mut cmd = tokio::process::Command::new("nice");
            let mut nice_args = vec!["-n".to_string(), "19".to_string(), "ffmpeg".to_string()];
            nice_args.extend(args);
            cmd.args(&nice_args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped());
            let result = crate::process::status_with_timeout(
                &mut cmd,
                std::time::Duration::from_secs(600),
            ).await;

            if matches!(result, Ok(s) if s.success()) {
                tracing::info!(
                    encoder = %hw.video_encoder,
                    "GPU video transcode succeeded"
                );
                return true;
            }

            if !fallback_to_cpu {
                tracing::error!(
                    encoder = %hw.video_encoder,
                    "GPU video transcode failed and CPU fallback disabled"
                );
                return false;
            }

            tracing::warn!(
                encoder = %hw.video_encoder,
                "GPU video transcode failed, retrying with CPU (libx264)"
            );
            // Remove partial output before retry
            let _ = tokio::fs::remove_file(output).await;
        }
    }

    // CPU fallback (original path)
    let mut cmd = tokio::process::Command::new("nice");
    cmd.args([
            "-n", "19",
            "ffmpeg", "-y",
            "-i", input,
            "-vf", "scale=trunc(iw*sar/2)*2:trunc(ih/2)*2,setsar=1",
            "-c:v", "libx264",
            "-preset", "medium",
            "-crf", "20",
            "-c:a", "aac",
            "-b:a", "192k",
            "-movflags", "+faststart",
            output,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let status = crate::process::status_with_timeout(
        &mut cmd,
        std::time::Duration::from_secs(600),
    ).await;

    matches!(status, Ok(s) if s.success())
}

/// Audio → MP3 (LAME).
async fn convert_audio(input: &str, output: &str) -> bool {
    let mut cmd = tokio::process::Command::new("nice");
    cmd.args([
            "-n", "19",
            "ffmpeg", "-y",
            "-i", input,
            "-codec:a", "libmp3lame",
            "-b:a", "192k",
            output,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let status = crate::process::status_with_timeout(
        &mut cmd,
        std::time::Duration::from_secs(600),
    ).await;

    matches!(status, Ok(s) if s.success())
}

// ── Conversion status endpoint ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ConversionStatusResponse {
    pub active: bool,
    pub total: i64,
    pub done: i64,
}

/// GET /api/admin/conversion-status
pub async fn conversion_status(
    _auth: AuthUser,
) -> Result<Json<ConversionStatusResponse>, AppError> {
    let (active, total, done) = progress_snapshot();
    Ok(Json(ConversionStatusResponse { active, total, done }))
}
