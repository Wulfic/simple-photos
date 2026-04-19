//! GPU-accelerated transcoding module.
//!
//! Detects hardware video acceleration at startup (NVENC, QSV, VAAPI, AMF)
//! and builds FFmpeg command lines that use GPU encoding when available.
//! Falls back to CPU (libx264) seamlessly if no GPU is detected or if a
//! GPU transcode fails at runtime.

pub mod ffmpeg_gpu;
pub mod gpu_probe;
pub mod handlers;

pub use gpu_probe::HwAccelCapability;
