//! Metadata-only edit save — the "Save" action in the editing engine.
//!
//! `PUT /api/photos/:id/crop` stores the edit parameters (crop rect,
//! rotation, brightness, trim) as a JSON string in `photos.crop_metadata`.
//! The original file is **never modified** — edits are applied visually by
//! client CSS transforms (web) or Compose transforms (Android).

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

/// Request body for `PUT /api/photos/{id}/crop`.
///
/// `crop_metadata` is a JSON string describing edits:
/// `{"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 0}`.
/// Send `null` to clear all edits.
#[derive(Debug, Deserialize)]
pub struct SetCropRequest {
    pub crop_metadata: Option<String>,
}

/// PUT /api/photos/:id/crop
///
/// Set (or clear) crop metadata for a photo.  This is a non-destructive,
/// metadata-only save — the original file on disk is never touched.
pub async fn set_crop(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(photo_id): Path<String>,
    Json(req): Json<SetCropRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate crop_metadata is valid JSON if provided, and limit size
    if let Some(ref crop) = req.crop_metadata {
        let crop = sanitize::sanitize_freeform(crop, 1024);
        if serde_json::from_str::<serde_json::Value>(&crop).is_err() {
            return Err(AppError::BadRequest(
                "crop_metadata must be valid JSON".into(),
            ));
        }
    }

    let rows = sqlx::query("UPDATE photos SET crop_metadata = ? WHERE id = ? AND user_id = ?")
        .bind(
            req.crop_metadata
                .as_ref()
                .map(|c| sanitize::sanitize_freeform(c, 1024)),
        )
        .bind(&photo_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(AppError::NotFound);
    }

    tracing::info!(
        "[editing/save] Set crop_metadata for photo_id={}: has_crop={}, raw={:?}",
        photo_id,
        req.crop_metadata.is_some(),
        req.crop_metadata.as_deref().unwrap_or("null"),
    );

    audit::log(
        &state,
        AuditEvent::PhotoCropSet,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "photo_id": photo_id,
            "has_crop": req.crop_metadata.is_some(),
        })),
    )
    .await;

    Ok(Json(serde_json::json!({
        "id": photo_id,
        "crop_metadata": req.crop_metadata,
    })))
}

/// Lightweight record returned by the crop-sync endpoint.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CropSyncRecord {
    pub id: String,
    pub crop_metadata: Option<String>,
}

/// GET /api/photos/crop-sync
///
/// Returns `{id, crop_metadata}` for **all** of the user's photos.
/// Android clients poll this during periodic sync so non-destructive edits
/// made on the web (or another device) are reflected locally.
pub async fn crop_sync(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<CropSyncRecord>>, AppError> {
    let records = sqlx::query_as::<_, CropSyncRecord>(
        "SELECT id, crop_metadata FROM photos WHERE user_id = ?",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.read_pool)
    .await?;

    Ok(Json(records))
}
