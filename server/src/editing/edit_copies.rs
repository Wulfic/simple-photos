//! Edit copies — lightweight metadata-only "versions" of photos.
//!
//! Each edit copy stores crop parameters, filters, etc. as JSON in the
//! `edit_copies` table **without** duplicating the underlying file or creating
//! a new `photos` row.  This is distinct from the "Save As Copy" path
//! ([`super::save_copy`]) which creates a fully independent rendered file.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::state::AppState;

/// Request body for `POST /api/photos/{id}/copies`.
#[derive(Debug, Deserialize)]
pub struct CreateEditCopyRequest {
    pub name: Option<String>,
    pub edit_metadata: String,
}

/// POST /api/photos/:id/copies — create a metadata-only "copy".
pub async fn create_edit_copy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(req): Json<CreateEditCopyRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify the photo belongs to this user
    let exists: bool =
        sqlx::query_scalar("SELECT COUNT(*) > 0 FROM photos WHERE id = ? AND user_id = ?")
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
        return Err(AppError::BadRequest(
            "edit_metadata must be valid JSON".into(),
        ));
    }

    let copy_id = Uuid::new_v4().to_string();
    let name = req
        .name
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

/// GET /api/photos/:id/copies — list all edit copies for a photo.
pub async fn list_edit_copies(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT id, name, edit_metadata, created_at FROM edit_copies \
         WHERE photo_id = ? AND user_id = ? ORDER BY created_at DESC",
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

/// DELETE /api/photos/:id/copies/:copy_id — delete a single edit copy.
pub async fn delete_edit_copy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((photo_id, copy_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = sqlx::query("DELETE FROM edit_copies WHERE id = ? AND photo_id = ? AND user_id = ?")
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
