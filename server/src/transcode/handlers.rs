//! Transcode status API endpoint.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::gpu_probe::HwAccelType;

#[derive(Debug, Serialize)]
pub struct TranscodeStatusResponse {
    pub gpu_available: bool,
    pub accel_type: HwAccelType,
    pub video_encoder: String,
    pub device: Option<String>,
    pub gpu_enabled: bool,
    pub fallback_to_cpu: bool,
}

/// GET /api/transcode/status — report GPU transcode capability.
pub async fn transcode_status(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<TranscodeStatusResponse>, AppError> {
    let hw = &state.hw_accel;
    Ok(Json(TranscodeStatusResponse {
        gpu_available: hw.is_gpu(),
        accel_type: hw.accel_type,
        video_encoder: hw.video_encoder.clone(),
        device: hw.device.clone(),
        gpu_enabled: state.config.transcode.gpu_enabled,
        fallback_to_cpu: state.config.transcode.gpu_fallback_to_cpu,
    }))
}
