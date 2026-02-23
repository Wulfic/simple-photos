//! Encryption settings and migration progress endpoints.

use axum::extract::State;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::EncryptionSettingsResponse;

/// GET /api/settings/encryption
/// Returns the current encryption mode and migration status.
pub async fn get_encryption_settings(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<EncryptionSettingsResponse>, AppError> {
    let mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    let (status, total, completed, error): (String, i64, i64, Option<String>) =
        sqlx::query_as(
            "SELECT status, total, completed, error FROM encryption_migration WHERE id = 'singleton'",
        )
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or_else(|| ("idle".to_string(), 0, 0, None));

    Ok(Json(EncryptionSettingsResponse {
        encryption_mode: mode,
        migration_status: status,
        migration_total: total,
        migration_completed: completed,
        migration_error: error,
    }))
}

/// PUT /api/admin/encryption
/// Toggle encryption mode. Admin only. Triggers background migration.
#[derive(Debug, Deserialize)]
pub struct SetEncryptionModeRequest {
    pub mode: String, // "plain" or "encrypted"
}

pub async fn set_encryption_mode(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SetEncryptionModeRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify admin
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    if req.mode != "plain" && req.mode != "encrypted" {
        return Err(AppError::BadRequest(
            "Mode must be 'plain' or 'encrypted'".into(),
        ));
    }

    let current: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    if current == req.mode {
        return Ok(Json(serde_json::json!({
            "message": format!("Already in '{}' mode", req.mode),
            "mode": req.mode,
        })));
    }

    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "idle".to_string());

    if mig_status != "idle" {
        return Err(AppError::BadRequest(
            "A migration is already in progress. Wait for it to complete.".into(),
        ));
    }

    sqlx::query(
        "INSERT OR REPLACE INTO server_settings (key, value) VALUES ('encryption_mode', ?)",
    )
    .bind(&req.mode)
    .execute(&state.pool)
    .await?;

    let direction = if req.mode == "encrypted" {
        "encrypting"
    } else {
        "decrypting"
    };

    let count: i64 = if req.mode == "encrypted" {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM photos WHERE user_id = ? AND encrypted_blob_id IS NULL",
        )
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM blobs WHERE user_id = ? AND blob_type IN ('photo', 'gif', 'video') \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items)",
        )
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?
    };

    // If there's nothing to migrate, stay idle — just flip the mode
    if count == 0 {
        tracing::info!(
            "Encryption mode changed to '{}'. No items to migrate.",
            req.mode
        );
        return Ok(Json(serde_json::json!({
            "message": format!("Encryption mode set to '{}'. No migration needed.", req.mode),
            "mode": req.mode,
            "migration_items": 0,
        })));
    }

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE encryption_migration SET status = ?, total = ?, completed = 0, started_at = ?, error = NULL WHERE id = 'singleton'",
    )
    .bind(direction)
    .bind(count)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    tracing::info!(
        "Encryption mode changed to '{}'. Migration: {} items.",
        req.mode,
        count
    );

    Ok(Json(serde_json::json!({
        "message": format!("Encryption mode set to '{}'. Migration started.", req.mode),
        "mode": req.mode,
        "migration_items": count,
    })))
}

/// POST /api/admin/encryption/progress
/// Client reports migration progress (one item at a time).
#[derive(Debug, Deserialize)]
pub struct MigrationProgressRequest {
    pub completed_count: i64,
    pub error: Option<String>,
    pub done: Option<bool>,
}

pub async fn report_migration_progress(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<MigrationProgressRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if let Some(ref err) = req.error {
        sqlx::query(
            "UPDATE encryption_migration SET error = ?, completed = ? WHERE id = 'singleton'",
        )
        .bind(err)
        .bind(req.completed_count)
        .execute(&state.pool)
        .await?;
    } else if req.done.unwrap_or(false) {
        sqlx::query(
            "UPDATE encryption_migration SET status = 'idle', completed = total, error = NULL WHERE id = 'singleton'",
        )
        .execute(&state.pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE encryption_migration SET completed = ? WHERE id = 'singleton'",
        )
        .bind(req.completed_count)
        .execute(&state.pool)
        .await?;
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}
