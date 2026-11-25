//! Admin endpoints for [`super::orphan_thumbs`]. Every call is a dry run unless
//! the body says `"apply": true`; each applied call is audited.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

use super::orphan_thumbs::{self, PurgeReport, RestoreReport, SweepOptions, SweepReport};
use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub struct SweepRequest {
    #[serde(default)]
    pub apply: bool,
    pub confirm_count: Option<u64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct RunRequest {
    pub run_id: String,
    #[serde(default)]
    pub apply: bool,
    pub confirm_count: Option<u64>,
}

/// `POST /api/admin/maintenance/orphan-thumbnails`
pub async fn sweep_orphan_thumbnails(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<SweepRequest>,
) -> Result<Json<SweepReport>, AppError> {
    require_admin(&state, &auth).await?;
    let root = (**state.storage_root.load()).clone();
    let report = orphan_thumbs::sweep(
        &state.pool,
        &root,
        SweepOptions {
            apply: req.apply,
            confirm_count: req.confirm_count,
            limit: req.limit,
        },
    )
    .await?;
    if req.apply {
        audit::log(
            &state,
            AuditEvent::AdminAction,
            Some(&auth.user_id),
            &headers,
            Some(serde_json::json!({
                "action": "orphan_thumbnails_quarantine",
                "run_id": report.run_id,
                "quarantined": report.quarantined,
                "quarantined_bytes": report.quarantined_bytes,
                "failed": report.failed,
                "remaining": report.remaining,
            })),
        )
        .await;
    }
    Ok(Json(report))
}

/// `POST /api/admin/maintenance/orphan-thumbnails/restore`
pub async fn restore_orphan_thumbnails(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<RunRequest>,
) -> Result<Json<RestoreReport>, AppError> {
    require_admin(&state, &auth).await?;
    let root = (**state.storage_root.load()).clone();
    let report = orphan_thumbs::restore(&state.pool, &root, &req.run_id, req.apply).await?;
    if req.apply {
        audit::log(
            &state,
            AuditEvent::AdminAction,
            Some(&auth.user_id),
            &headers,
            Some(serde_json::json!({
                "action": "orphan_thumbnails_restore",
                "run_id": report.run_id,
                "restored": report.restored,
                "conflicts": report.conflicts,
                "failed": report.failed,
            })),
        )
        .await;
    }
    Ok(Json(report))
}

/// `POST /api/admin/maintenance/orphan-thumbnails/purge`
pub async fn purge_orphan_thumbnails(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<RunRequest>,
) -> Result<Json<PurgeReport>, AppError> {
    require_admin(&state, &auth).await?;
    let root = (**state.storage_root.load()).clone();
    let report = orphan_thumbs::purge(
        &state.pool,
        &root,
        &req.run_id,
        req.apply,
        req.confirm_count,
    )
    .await?;
    if req.apply {
        audit::log(
            &state,
            AuditEvent::AdminAction,
            Some(&auth.user_id),
            &headers,
            Some(serde_json::json!({
                "action": "orphan_thumbnails_purge",
                "run_id": report.run_id,
                "purged": report.purged,
                "bytes": report.bytes,
                "skipped_live": report.skipped_live,
                "failed": report.failed,
            })),
        )
        .await;
    }
    Ok(Json(report))
}
