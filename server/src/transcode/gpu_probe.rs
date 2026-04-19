//! GPU hardware acceleration detection via FFmpeg probing.
//!
//! Runs `ffmpeg -hwaccels` and `ffmpeg -encoders` at startup to determine
//! which hardware acceleration backends are available.  Results are cached
//! for the lifetime of the process (GPU capabilities don't change at runtime).

use std::process::Command;

/// Type of hardware acceleration backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HwAccelType {
    /// NVIDIA NVENC/NVDEC (CUDA)
    Nvenc,
    /// Intel Quick Sync Video
    Qsv,
    /// Video Acceleration API (Intel/AMD on Linux)
    Vaapi,
    /// AMD Advanced Media Framework (Windows)
    Amf,
    /// Software-only (libx264)
    Cpu,
}

impl std::fmt::Display for HwAccelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nvenc => write!(f, "nvenc"),
            Self::Qsv => write!(f, "qsv"),
            Self::Vaapi => write!(f, "vaapi"),
            Self::Amf => write!(f, "amf"),
            Self::Cpu => write!(f, "cpu"),
        }
    }
}

/// Detected hardware acceleration capability.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HwAccelCapability {
    /// Which acceleration backend is active.
    pub accel_type: HwAccelType,
    /// FFmpeg video encoder name (e.g. "h264_nvenc", "libx264").
    pub video_encoder: String,
    /// Optional hardware device path (e.g. "/dev/dri/renderD128" for VAAPI).
    pub device: Option<String>,
}

impl HwAccelCapability {
    /// Returns true if this is a GPU-accelerated backend (not CPU).
    pub fn is_gpu(&self) -> bool {
        self.accel_type != HwAccelType::Cpu
    }

    /// CPU-only fallback capability.
    pub fn cpu_fallback() -> Self {
        Self {
            accel_type: HwAccelType::Cpu,
            video_encoder: "libx264".into(),
            device: None,
        }
    }
}

/// Probe the system for GPU hardware acceleration via FFmpeg.
///
/// Priority order: NVENC > QSV > VAAPI > AMF > CPU.
/// Each candidate is verified by checking that the corresponding
/// FFmpeg encoder is available.
///
/// If `device_override` is non-empty, it is used as the GPU device path
/// (e.g. "/dev/dri/renderD128") instead of auto-detection.
pub fn probe_hwaccel(gpu_enabled: bool, device_override: &str) -> HwAccelCapability {
    if !gpu_enabled {
        tracing::info!("GPU transcoding disabled by config");
        return HwAccelCapability::cpu_fallback();
    }

    // Check if ffmpeg is available at all
    let hwaccels = match run_ffmpeg_hwaccels() {
        Some(output) => output,
        None => {
            tracing::warn!("FFmpeg not found or failed to list hwaccels — using CPU");
            return HwAccelCapability::cpu_fallback();
        }
    };

    let encoders = run_ffmpeg_encoders().unwrap_or_default();

    tracing::debug!(
        hwaccels = %hwaccels.lines().take(20).collect::<Vec<_>>().join(", "),
        "GPU probe: available FFmpeg hardware accelerators"
    );
    tracing::debug!(
        has_h264_nvenc = encoders.contains("h264_nvenc"),
        has_h264_qsv = encoders.contains("h264_qsv"),
        has_h264_vaapi = encoders.contains("h264_vaapi"),
        has_h264_amf = encoders.contains("h264_amf"),
        "GPU probe: hardware encoder availability"
    );

    // Priority: NVENC > QSV > VAAPI > AMF
    if hwaccels.contains("cuda") && encoders.contains("h264_nvenc") {
        tracing::info!("GPU transcode: detected NVIDIA NVENC (h264_nvenc)");
        return HwAccelCapability {
            accel_type: HwAccelType::Nvenc,
            video_encoder: "h264_nvenc".into(),
            device: None,
        };
    }

    if hwaccels.contains("qsv") && encoders.contains("h264_qsv") {
        tracing::info!("GPU transcode: detected Intel QSV (h264_qsv)");
        return HwAccelCapability {
            accel_type: HwAccelType::Qsv,
            video_encoder: "h264_qsv".into(),
            device: None,
        };
    }

    if hwaccels.contains("vaapi") && encoders.contains("h264_vaapi") {
        // Use configured device or auto-detect
        let device = if !device_override.is_empty() {
            tracing::info!(
                device = %device_override,
                "GPU transcode: using configured VAAPI device"
            );
            Some(device_override.to_string())
        } else {
            find_vaapi_device()
        };
        tracing::info!(
            device = ?device,
            "GPU transcode: detected VAAPI (h264_vaapi)"
        );
        return HwAccelCapability {
            accel_type: HwAccelType::Vaapi,
            video_encoder: "h264_vaapi".into(),
            device,
        };
    }

    if (hwaccels.contains("d3d11va") || hwaccels.contains("amf"))
        && encoders.contains("h264_amf")
    {
        tracing::info!("GPU transcode: detected AMD AMF (h264_amf)");
        return HwAccelCapability {
            accel_type: HwAccelType::Amf,
            video_encoder: "h264_amf".into(),
            device: None,
        };
    }

    tracing::info!("GPU transcode: no hardware acceleration detected — using CPU (libx264)");
    HwAccelCapability::cpu_fallback()
}

/// Run `ffmpeg -hwaccels` and return the raw stdout.
fn run_ffmpeg_hwaccels() -> Option<String> {
    let output = Command::new("ffmpeg")
        .args(["-hwaccels", "-hide_banner"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_lowercase())
    } else {
        None
    }
}

/// Run `ffmpeg -encoders` and return the raw stdout.
fn run_ffmpeg_encoders() -> Option<String> {
    let output = Command::new("ffmpeg")
        .args(["-encoders", "-hide_banner"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_lowercase())
    } else {
        None
    }
}

/// Find the first usable VAAPI render device.
fn find_vaapi_device() -> Option<String> {
    // Common Linux render device paths
    for path in &["/dev/dri/renderD128", "/dev/dri/renderD129"] {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    None
}
