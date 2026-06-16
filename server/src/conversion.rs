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
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::OnceLock;

use axum::Json;
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::transcode::HwAccelCapability;

// ── GPU acceleration config (set once at startup) ────────────────────────────

/// Cached GPU hardware acceleration capability, set by `init_gpu_config()`.
static GPU_CONFIG: OnceLock<GpuConversionConfig> = OnceLock::new();

struct GpuConversionConfig {
    hwaccel: HwAccelCapability,
    fallback_to_cpu: bool,
}

/// Initialize GPU conversion config. Called once from main.rs at startup.
pub fn init_gpu_config(hwaccel: HwAccelCapability, fallback_to_cpu: bool) {
    let _ = GPU_CONFIG.set(GpuConversionConfig {
        hwaccel,
        fallback_to_cpu,
    });
}

/// Public accessor for the active hardware-acceleration capability.
/// Returns `None` when `init_gpu_config` has not been called yet.
/// Used by the web-preview pipeline so on-the-fly mp4 transcodes
/// honour the same NVENC/QSV/VAAPI path as bulk conversion.
pub fn active_hwaccel() -> Option<&'static HwAccelCapability> {
    GPU_CONFIG.get().map(|c| &c.hwaccel)
}

/// Public accessor for the configured CPU-fallback policy.
pub fn cpu_fallback_enabled() -> bool {
    GPU_CONFIG.get().map(|c| c.fallback_to_cpu).unwrap_or(true)
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

/// Register an additional `n` items to the in-flight conversion total
/// **without** resetting the existing `done` counter.  Used by the
/// per-upload conversion path (`photos/upload.rs`) where each upload is
/// its own one-item "batch" but we want the banner to span all
/// concurrent uploads instead of flashing once per file.
///
/// Safe to interleave with `progress_start` (batch ingest) — the
/// running totals just accumulate, and the banner naturally hides once
/// `done == total` and `active` flips back to false via
/// [`progress_finish_one`].
pub fn progress_add(n: i64) {
    CONV_TOTAL.fetch_add(n, Ordering::Relaxed);
    CONV_ACTIVE.store(true, Ordering::Relaxed);
}

/// Counterpart to [`progress_add`] — increments `done` and clears the
/// `active` flag once `done` has caught up to `total`.  This is what
/// keeps the banner visible across many concurrent uploads but lets it
/// hide when the queue drains.
pub fn progress_finish_one() {
    let done = CONV_DONE.fetch_add(1, Ordering::Relaxed) + 1;
    let total = CONV_TOTAL.load(Ordering::Relaxed);
    if done >= total {
        CONV_ACTIVE.store(false, Ordering::Relaxed);
    }
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

/// Conversion ordering priority (lower runs first).
///
/// Image and audio conversions finish in well under a second; a single video
/// transcode can take minutes (GPU attempt + CPU fallback, each capped at the
/// 600 s ffmpeg timeout). The ingest pass is sequential, so enumerating videos
/// first makes a mixed import look frozen on the first big file (#10). Ordering
/// fast formats ahead of videos keeps progress visibly moving and pushes the
/// slow transcodes to the end of the batch.
pub fn conversion_priority(cat: MediaCategory) -> u8 {
    match cat {
        MediaCategory::Image => 0,
        MediaCategory::Audio => 1,
        MediaCategory::Video => 2,
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
/// Image conversion is FFmpeg-only (no ImageMagick). HEIC/HEIF decodes natively
/// via FFmpeg's mov demuxer + HEVC decoder; RAW camera formats are unsupported.
///
/// For video conversions, uses GPU-accelerated encoding when available
/// (configured via `init_gpu_config()` at startup).
pub async fn convert_file(
    input: &Path,
    output: &Path,
    target: &ConversionTarget,
) -> Result<(), String> {
    // ── Path-injection sanitizer ─────────────────────────────────────────
    // Canonicalize input (must already exist) and the output's parent so all
    // subsequent filesystem operations work against fully resolved paths.
    // We then verify the supplied paths cannot escape those canonical
    // ancestors. This is the standard barrier for the rust/path-injection
    // CodeQL query and is defense-in-depth on top of caller-side validation.
    let canonical_input = tokio::fs::canonicalize(input)
        .await
        .map_err(|e| format!("Canonicalize input path: {e}"))?;

    let output_parent = output
        .parent()
        .ok_or("Output path has no parent directory")?;
    tokio::fs::create_dir_all(output_parent)
        .await
        .map_err(|e| format!("Create output directory: {e}"))?;
    let canonical_output_parent = tokio::fs::canonicalize(output_parent)
        .await
        .map_err(|e| format!("Canonicalize output directory: {e}"))?;
    let output_file_name = output
        .file_name()
        .ok_or("Output path has no file name component")?;
    let canonical_output = canonical_output_parent.join(output_file_name);
    if !canonical_output.starts_with(&canonical_output_parent) {
        return Err("Output path escapes its parent directory".into());
    }

    let input_str = canonical_input
        .to_str()
        .ok_or("Invalid input path encoding")?;
    let output_str = canonical_output
        .to_str()
        .ok_or("Invalid output path encoding")?;

    let success = match target.category {
        MediaCategory::Image => convert_image(input_str, output_str).await,
        MediaCategory::Video => {
            let gpu = gpu_config();
            convert_video(
                input_str,
                output_str,
                gpu.map(|g| &g.hwaccel),
                gpu.map(|g| g.fallback_to_cpu).unwrap_or(true),
            )
            .await
        }
        MediaCategory::Audio => convert_audio(input_str, output_str).await,
    };

    if !success {
        let _ = tokio::fs::remove_file(&canonical_output).await;
        return Err(format!(
            "Conversion failed for '{}' → .{}",
            canonical_input
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?"),
            target.extension,
        ));
    }

    // Verify the output file exists and is non-empty.
    match tokio::fs::metadata(&canonical_output).await {
        Ok(m) if m.len() > 0 => Ok(()),
        Ok(_) => {
            let _ = tokio::fs::remove_file(&canonical_output).await;
            Err("Conversion produced an empty file".into())
        }
        Err(e) => Err(format!("Output file missing after conversion: {e}")),
    }
}

// ── Format-specific converters ───────────────────────────────────────────────

/// Image → JPEG via FFmpeg.
///
/// FFmpeg is the *only* image converter we depend on — no ImageMagick — to keep
/// the install to a single media tool. HEIC/HEIF (Apple's default camera format)
/// decodes natively here: FFmpeg reads the ISOBMFF/HEIF container via its `mov`
/// demuxer and decodes the still image with the built-in HEVC decoder, so no
/// libheif build flag is needed. (RAW camera formats are intentionally
/// unsupported — they'd require per-vendor decoders we don't ship.)
async fn convert_image(input: &str, output: &str) -> bool {
    tracing::debug!(input = %input, output = %output, "Image conversion: starting JPEG conversion");
    // FFmpeg: high-quality JPEG output (-q:v 2 ≈ 92% quality).
    let mut cmd = crate::process::background_command("ffmpeg");
    cmd.args(["-y", "-i", input, "-q:v", "2", output])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let ffmpeg =
        crate::process::status_with_timeout(&mut cmd, std::time::Duration::from_secs(600)).await;

    let ok = matches!(ffmpeg, Ok(s) if s.success());
    if ok {
        tracing::debug!(input = %input, "Image conversion: FFmpeg JPEG conversion succeeded");
    } else {
        tracing::warn!(input = %input, "Image conversion failed");
    }
    ok
}

/// Video → MP4 (H.264 + AAC).  Quality-tuned for clarity at reasonable sizes.
/// When a GPU `hwaccel` capability is provided, uses hardware-accelerated
/// encoding.  Falls back to CPU (libx264) if the GPU transcode fails and
/// `fallback_to_cpu` is true.
pub(crate) async fn convert_video(
    input: &str,
    output: &str,
    hwaccel: Option<&HwAccelCapability>,
    fallback_to_cpu: bool,
) -> bool {
    // Diagnostics: log exactly which path was selected for every video.
    // Without this, operators report "still using CPU!" and we have no
    // way to tell whether the GPU branch was even considered.
    match hwaccel {
        Some(hw) if hw.is_gpu() => tracing::debug!(
            input = %input,
            encoder = %hw.video_encoder,
            "convert_video: GPU path selected"
        ),
        Some(_) => tracing::warn!(
            input = %input,
            "convert_video: hwaccel registered as CPU (probe found no GPU encoder)"
        ),
        None => tracing::warn!(
            input = %input,
            "convert_video: no hwaccel config registered — init_gpu_config not called?"
        ),
    }

    // Try GPU path first if available
    if let Some(hw) = hwaccel {
        if hw.is_gpu() {
            let args = crate::transcode::ffmpeg_gpu::build_video_transcode_args(input, output, hw);
            tracing::info!(
                encoder = %hw.video_encoder,
                accel = %hw.accel_type,
                device = ?hw.device,
                input = %input,
                output = %output,
                "GPU transcode: starting hardware-accelerated video conversion"
            );
            tracing::debug!(
                ffmpeg_args = ?args,
                "GPU transcode: FFmpeg command arguments"
            );
            let gpu_start = std::time::Instant::now();
            let mut cmd = crate::process::background_command("ffmpeg");
            cmd.args(&args).stdout(std::process::Stdio::null());
            let result =
                crate::process::run_with_timeout(&mut cmd, std::time::Duration::from_secs(600))
                    .await;

            let gpu_ok = matches!(&result, Ok(out) if out.status.success());

            if gpu_ok {
                tracing::info!(
                    encoder = %hw.video_encoder,
                    elapsed_ms = gpu_start.elapsed().as_millis(),
                    input = %input,
                    "GPU transcode: hardware-accelerated conversion succeeded"
                );
                return true;
            }

            // Log the actual FFmpeg error so operators can diagnose failures.
            let ffmpeg_stderr = match &result {
                Ok(out) => String::from_utf8_lossy(&out.stderr).to_string(),
                Err(e) => e.clone(),
            };
            let last_lines: String = ffmpeg_stderr
                .lines()
                .rev()
                .take(10)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");

            if !fallback_to_cpu {
                tracing::error!(
                    encoder = %hw.video_encoder,
                    elapsed_ms = gpu_start.elapsed().as_millis(),
                    ffmpeg_error = %last_lines,
                    "GPU transcode: hardware conversion failed and CPU fallback is disabled"
                );
                return false;
            }

            tracing::warn!(
                encoder = %hw.video_encoder,
                elapsed_ms = gpu_start.elapsed().as_millis(),
                ffmpeg_error = %last_lines,
                "GPU transcode: hardware conversion failed — retrying with CPU libx264"
            );
            // Remove partial output before retry
            let _ = tokio::fs::remove_file(output).await; // codeql[rust/path-injection] -- path is server temp dir + UUID; ext restricted to alphanumeric at call sites
        }
    }

    // CPU fallback (original path)
    tracing::info!(
        input = %input,
        output = %output,
        encoder = "libx264",
        "GPU transcode: running CPU software encoding"
    );
    let cpu_start = std::time::Instant::now();
    let mut cmd = crate::process::background_command("ffmpeg");
    cmd.args([
        "-y",
        "-i",
        input,
        "-vf",
        "scale=trunc(iw*sar/2)*2:trunc(ih/2)*2,setsar=1",
        "-c:v",
        "libx264",
        "-preset",
        "medium",
        "-crf",
        "20",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-movflags",
        "+faststart",
        output,
    ])
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    let status =
        crate::process::status_with_timeout(&mut cmd, std::time::Duration::from_secs(600)).await;
    let ok = matches!(status, Ok(s) if s.success());
    if ok {
        tracing::info!(
            input = %input,
            elapsed_ms = cpu_start.elapsed().as_millis(),
            "GPU transcode: CPU software encoding succeeded"
        );
    } else {
        tracing::error!(
            input = %input,
            elapsed_ms = cpu_start.elapsed().as_millis(),
            "GPU transcode: CPU software encoding failed"
        );
    }
    ok
}

/// Audio → MP3 (LAME).
async fn convert_audio(input: &str, output: &str) -> bool {
    tracing::debug!(input = %input, output = %output, "Audio conversion: starting MP3 conversion");
    tracing::debug!(input = %input, output = %output, "Audio conversion: starting MP3 conversion");
    let mut cmd = crate::process::background_command("ffmpeg");
    cmd.args([
        "-y",
        "-i",
        input,
        "-codec:a",
        "libmp3lame",
        "-b:a",
        "192k",
        output,
    ])
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    let status =
        crate::process::status_with_timeout(&mut cmd, std::time::Duration::from_secs(600)).await;

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
    Ok(Json(ConversionStatusResponse {
        active,
        total,
        done,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_priority_orders_videos_last() {
        // Fast formats first, slow video transcodes last (#10).
        assert!(
            conversion_priority(MediaCategory::Image) < conversion_priority(MediaCategory::Video)
        );
        assert!(
            conversion_priority(MediaCategory::Audio) < conversion_priority(MediaCategory::Video)
        );
    }

    #[test]
    fn conversion_priority_sorts_a_mixed_batch_fast_first() {
        let mut cats = vec![
            MediaCategory::Video,
            MediaCategory::Image,
            MediaCategory::Video,
            MediaCategory::Audio,
            MediaCategory::Image,
        ];
        // Stable sort preserves discovery order within each tier.
        cats.sort_by_key(|c| conversion_priority(*c));
        assert_eq!(
            cats,
            vec![
                MediaCategory::Image,
                MediaCategory::Image,
                MediaCategory::Audio,
                MediaCategory::Video,
                MediaCategory::Video,
            ]
        );
    }
}
