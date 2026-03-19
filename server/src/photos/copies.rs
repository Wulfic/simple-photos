//! Photo duplication and edit copy management endpoints.
//!
//! **Duplicate photo** (`POST /api/photos/:id/duplicate`):
//! Creates a new `photos` row that shares the same underlying file.
//! Used by the "Save Copy" feature in the editor — the copy has its
//! own metadata (crop, favorites, tags) but no extra disk usage.
//!
//! **Edit copies** (`POST/GET/DELETE /api/photos/:id/copies`):
//! Lightweight metadata-only "versions" stored as JSON in the `edit_copies`
//! table. Each copy records crop parameters, filters, etc. without
//! duplicating the file or photos row.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

use super::models::Photo;
use super::utils::utc_now_iso;

// ── Duplicate Photo (Save as Copy) ─────────────────────────────────────────

/// Request body for `POST /api/photos/{id}/duplicate`.
/// Creates a new photos row sharing the same underlying file but with
/// independent crop/edit metadata. `crop_metadata` is optional JSON.
#[derive(Debug, Deserialize)]
pub struct DuplicatePhotoRequest {
    pub crop_metadata: Option<String>,
}

/// POST /api/photos/:id/duplicate — create a new photos row that shares the
/// same underlying file but carries its own crop/edit metadata.
pub async fn duplicate_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(req): Json<DuplicatePhotoRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Fetch the original photo
    let original: Option<Photo> = sqlx::query_as(
        "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path, \
         created_at, encrypted_blob_id, encrypted_thumb_blob_id, is_favorite, \
         crop_metadata, camera_model, photo_hash \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let original = original.ok_or(AppError::NotFound)?;

    // Validate crop_metadata if provided
    let meta = req.crop_metadata.as_deref().map(|m| {
        let sanitized = sanitize::sanitize_freeform(m, 2048);
        if serde_json::from_str::<serde_json::Value>(&sanitized).is_err() {
            return Err(AppError::BadRequest("crop_metadata must be valid JSON".into()));
        }
        Ok(sanitized)
    }).transpose()?;

    let new_id = Uuid::new_v4().to_string();
    let now = utc_now_iso();

    // Build "Copy of <filename>" name
    let copy_filename = if original.filename.starts_with("Copy of ") {
        original.filename.clone()
    } else {
        format!("Copy of {}", original.filename)
    };

    let new_thumb_path = original.thumb_path.clone();

    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
         size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
         thumb_path, created_at, encrypted_blob_id, encrypted_thumb_blob_id, \
         is_favorite, crop_metadata, camera_model, photo_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?)",
    )
    .bind(&new_id)
    .bind(&auth.user_id)
    .bind(&copy_filename)
    .bind(&original.file_path)
    .bind(&original.mime_type)
    .bind(&original.media_type)
    .bind(original.size_bytes)
    .bind(original.width)
    .bind(original.height)
    .bind(original.duration_secs)
    .bind(&original.taken_at)
    .bind(original.latitude)
    .bind(original.longitude)
    .bind(&new_thumb_path)
    .bind(&now)
    .bind(&original.encrypted_blob_id)
    .bind(&original.encrypted_thumb_blob_id)
    .bind(&meta)
    .bind(&original.camera_model)
    // photo_hash must be NULL for copies — there is a UNIQUE index on
    // (user_id, photo_hash) WHERE photo_hash IS NOT NULL, so reusing the
    // original's hash would violate the constraint.
    .bind(None::<String>)
    .execute(&state.pool)
    .await?;

    tracing::info!(
        "Duplicated photo {} → {} for user {}",
        photo_id,
        new_id,
        auth.user_id
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": new_id,
            "source_photo_id": photo_id,
            "filename": copy_filename,
            "crop_metadata": meta.as_deref().and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok()),
        })),
    ))
}

// ── Edit Copies (Save Copy) ────────────────────────────────────────────────

/// Request body for `POST /api/photos/{id}/copies`.
/// Creates a metadata-only edit copy of a photo — stores the edit parameters
/// (brightness, rotation, filter, etc.) without duplicating the underlying file.
#[derive(Debug, Deserialize)]
pub struct CreateEditCopyRequest {
    pub name: Option<String>,
    pub edit_metadata: String,
}

/// POST /api/photos/:id/copies — create a metadata-only "copy" of a photo/video/audio
pub async fn create_edit_copy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(req): Json<CreateEditCopyRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify the photo belongs to this user
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    if !exists {
        return Err(AppError::NotFound);
    }

    // Validate edit_metadata is valid JSON
    let meta = sanitize::sanitize_freeform(&req.edit_metadata, 2048);
    if serde_json::from_str::<serde_json::Value>(&meta).is_err() {
        return Err(AppError::BadRequest("edit_metadata must be valid JSON".into()));
    }

    let copy_id = Uuid::new_v4().to_string();
    let name = req.name
        .as_deref()
        .map(|n| sanitize::sanitize_freeform(n, 128))
        .unwrap_or_else(|| {
            let now = Utc::now().format("%Y-%m-%d %H:%M").to_string();
            format!("Copy {}", now)
        });

    sqlx::query(
        "INSERT INTO edit_copies (id, photo_id, user_id, name, edit_metadata) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&copy_id)
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&name)
    .bind(&meta)
    .execute(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({
        "id": copy_id,
        "photo_id": photo_id,
        "name": name,
        "edit_metadata": serde_json::from_str::<serde_json::Value>(&meta).ok(),
    })))
}

/// GET /api/photos/:id/copies — list all edit copies for a photo
pub async fn list_edit_copies(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT id, name, edit_metadata, created_at FROM edit_copies WHERE photo_id = ? AND user_id = ? ORDER BY created_at DESC",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let copies: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, name, meta, created_at)| {
            serde_json::json!({
                "id": id,
                "name": name,
                "edit_metadata": serde_json::from_str::<serde_json::Value>(&meta).ok(),
                "created_at": created_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "copies": copies })))
}

/// DELETE /api/photos/:id/copies/:copy_id — delete a single edit copy
pub async fn delete_edit_copy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((photo_id, copy_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = sqlx::query(
        "DELETE FROM edit_copies WHERE id = ? AND photo_id = ? AND user_id = ?",
    )
    .bind(&copy_id)
    .bind(&photo_id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}
