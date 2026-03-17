//! Encryption settings and migration progress endpoints.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::sanitize;
use crate::setup::admin::require_admin;
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
    .fetch_optional(&state.read_pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    let (status, total, completed, error): (String, i64, i64, Option<String>) =
        sqlx::query_as(
            "SELECT status, total, completed, error FROM encryption_migration WHERE id = 'singleton'",
        )
        .fetch_optional(&state.read_pool)
        .await?
        .unwrap_or_else(|| ("idle".to_string(), 0, 0, None));

    tracing::info!(
        "[DIAG:ENCRYPT_SETTINGS] mode={}, status={}, progress={}/{}, error={}",
        mode, status, completed, total,
        error.as_deref().unwrap_or("none")
    );

    Ok(Json(EncryptionSettingsResponse {
        encryption_mode: mode,
        migration_status: status,
        migration_total: total,
        migration_completed: completed,
        migration_error: error,
    }))
}

/// PUT /api/admin/encryption
/// Toggle encryption mode. Admin only.
///
/// Sets the mode in `server_settings` and updates `encryption_migration`
/// status to `"encrypting"` or `"decrypting"`.
///
/// When `key_hex` is provided alongside `mode = "encrypted"`, the server:
/// 1. Validates and wraps the AES-256 key, persisting it in the DB
/// 2. Loads the key into in-memory `AppState` for immediate use
/// 3. Auto-starts the server-side encryption migration in the background
///
/// This makes the entire migration autonomous — the client only needs to
/// send the key once; even if the browser is closed, the server continues.
///
/// Without `key_hex`, the mode is set but actual migration must be started
/// separately via `POST /api/admin/encryption/migrate` or will be picked up
/// by the autoscan background task if a stored key is available.
///
/// Request body for `PUT /api/admin/encryption`.
#[derive(Debug, Deserialize)]
pub struct SetEncryptionModeRequest {
    /// Target encryption mode: `"plain"` or `"encrypted"`.
    pub mode: String,
    /// Optional: hex-encoded AES-256 key (64 hex chars).
    /// When provided with `mode = "encrypted"`, the server stores the key
    /// (wrapped by the JWT secret) and auto-starts server-side migration.
    pub key_hex: Option<String>,
}

pub async fn set_encryption_mode(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SetEncryptionModeRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    if req.mode != "plain" && req.mode != "encrypted" {
        return Err(AppError::BadRequest(
            "Mode must be 'plain' or 'encrypted'".into(),
        ));
    }

    let current: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.read_pool)
    .await?
    .unwrap_or_else(|| "plain".to_string());

    if current == req.mode && req.key_hex.is_none() {
        return Ok(Json(serde_json::json!({
            "message": format!("Already in '{}' mode", req.mode),
            "mode": req.mode,
        })));
    }

    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(&state.read_pool)
    .await?
    .unwrap_or_else(|| "idle".to_string());

    if mig_status != "idle" && req.key_hex.is_none() {
        return Err(AppError::BadRequest(
            "A migration is already in progress. Wait for it to complete.".into(),
        ));
    }

    // If a key is provided, validate and persist it (wrapped by the JWT secret)
    let parsed_key: Option<[u8; 32]> = if let Some(ref key_hex) = req.key_hex {
        let key = crate::crypto::parse_key_hex(key_hex)
            .map_err(|e| AppError::BadRequest(e))?;
        crate::crypto::store_wrapped_key(&state.pool, &key, &state.config.auth.jwt_secret)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to persist encryption key: {}", e)))?;
        // Also load it into the in-memory store for immediate use
        {
            let mut guard = state.encryption_key.write().await;
            *guard = Some(key);
        }
        Some(key)
    } else {
        None
    };

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

    // If there's nothing to migrate NOW, stay idle — just flip the mode.
    // The autoscan background task will detect new files and trigger migration
    // later once the initial scan completes (it calls auto_start_migration_if_needed).
    if count == 0 {
        tracing::info!(
            "Encryption mode changed to '{}'. No items to migrate yet (scan may find files later).",
            req.mode
        );
        return Ok(Json(serde_json::json!({
            "message": format!("Encryption mode set to '{}'. Migration will start automatically when photos are found.", req.mode),
            "mode": req.mode,
            "migration_items": 0,
        })));
    }

    let now = Utc::now().to_rfc3339();
    // Only start a new migration if status is idle (avoid overwriting a running migration)
    if mig_status == "idle" {
        sqlx::query(
            "UPDATE encryption_migration SET status = ?, total = ?, completed = 0, started_at = ?, error = NULL WHERE id = 'singleton'",
        )
        .bind(direction)
        .bind(count)
        .bind(&now)
        .execute(&state.pool)
        .await?;
    }

    tracing::info!(
        "Encryption mode changed to '{}'. Migration: {} items.",
        req.mode,
        count
    );

    // If we have the key and we're encrypting, auto-start the server-side migration
    if req.mode == "encrypted" && parsed_key.is_some() {
        let key = parsed_key.unwrap();
        let pool = state.pool.clone();
        // Lock-free read via ArcSwap.
        let storage_root = (**state.storage_root.load()).clone();
        let user_id = auth.user_id.clone();
        let convert_notify = state.convert_notify.clone();
        let encryption_key_store = state.encryption_key.clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();

        tokio::spawn(async move {
            super::server_migrate::run_migration_from_stored_key(
                key, user_id, pool, storage_root, convert_notify, encryption_key_store, jwt_secret,
            ).await;
        });

        tracing::info!("Server-side migration auto-started after setMode (key provided)");
    }

    Ok(Json(serde_json::json!({
        "message": format!("Encryption mode set to '{}'. Migration started.", req.mode),
        "mode": req.mode,
        "migration_items": count,
    })))
}

/// Request body for `POST /api/admin/encryption/progress`.
/// The client-side migration worker reports progress as it encrypts each photo.
/// When `done` is `true`, the server marks the migration as complete.
#[derive(Debug, Deserialize)]
pub struct MigrationProgressRequest {
    pub completed_count: i64,
    pub error: Option<String>,
    pub done: Option<bool>,
}

pub async fn report_migration_progress(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<MigrationProgressRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // This endpoint is under /api/admin/ and should not be callable by
    // non-admin users to prevent migration state corruption.
    require_admin(&state, &auth).await?;

    // Sanitize error messages — cap length and strip control characters
    let error = req.error.as_deref().map(|e| sanitize::sanitize_freeform(e, 2048));

    if let Some(ref err) = error {
        tracing::warn!(
            completed = req.completed_count,
            done = req.done.unwrap_or(false),
            error = %err,
            "Encryption migration progress: client reported error"
        );
    } else {
        tracing::debug!(
            completed = req.completed_count,
            done = req.done.unwrap_or(false),
            "Encryption migration progress update"
        );
    }

    if req.done.unwrap_or(false) {
        tracing::info!("[DIAG:ENCRYPT] report_migration_progress: done=true, setting migration to idle");
        // Migration finished — preserve error message if one was sent with done flag
        if let Some(ref err) = error {
            sqlx::query(
                "UPDATE encryption_migration SET status = 'idle', completed = total, error = ? WHERE id = 'singleton'",
            )
            .bind(err)
            .execute(&state.pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE encryption_migration SET status = 'idle', completed = total, error = NULL WHERE id = 'singleton'",
            )
            .execute(&state.pool)
            .await?;
        }

        // 5-second delay before triggering the converter — ensures all DB
        // writes from the migration have fully settled.
        let notify = state.convert_notify.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            notify.notify_one();
            tracing::info!("[DIAG:ENCRYPT] migration done — sent convert notify after 5s delay");
        });
        tracing::info!("[DIAG:ENCRYPT] migration done — scheduled convert notify in 5s");
    } else if let Some(ref err) = error {
        sqlx::query(
            "UPDATE encryption_migration SET error = ?, completed = ? WHERE id = 'singleton'",
        )
        .bind(err)
        .bind(req.completed_count)
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

/// POST /api/photos/{id}/mark-encrypted
/// Link a plain photo to its encrypted blob so it won't be re-migrated.
/// Also accepts an optional `thumb_blob_id` so the client-side migration
/// worker can set `encrypted_thumb_blob_id` in the same request.
#[derive(Debug, Deserialize)]
pub struct MarkEncryptedRequest {
    pub blob_id: String,
    /// Optional: the encrypted thumbnail blob ID. When provided, the server
    /// sets `encrypted_thumb_blob_id` on the photos row alongside
    /// `encrypted_blob_id`. This allows the client-side migration worker
    /// to fully populate both fields in a single call.
    pub thumb_blob_id: Option<String>,
}

pub async fn mark_photo_encrypted(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(req): Json<MarkEncryptedRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Verify the photo belongs to this user
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_one(&state.read_pool)
    .await?;

    if !exists {
        return Err(AppError::NotFound);
    }

    // Verify the blob belongs to this user
    let blob_exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&req.blob_id)
    .bind(&auth.user_id)
    .fetch_one(&state.read_pool)
    .await?;

    if !blob_exists {
        return Err(AppError::NotFound);
    }

    // If a thumb_blob_id is provided, verify it belongs to this user too
    if let Some(ref thumb_id) = req.thumb_blob_id {
        if !thumb_id.is_empty() {
            let thumb_exists: bool = sqlx::query_scalar(
                "SELECT COUNT(*) > 0 FROM blobs WHERE id = ? AND user_id = ?",
            )
            .bind(thumb_id)
            .bind(&auth.user_id)
            .fetch_one(&state.read_pool)
            .await?;

            if !thumb_exists {
                return Err(AppError::BadRequest(
                    "thumb_blob_id does not exist or does not belong to this user".into(),
                ));
            }
        }
    }

    // Determine the effective thumb_blob_id (None if empty or absent)
    let effective_thumb: Option<&str> = req
        .thumb_blob_id
        .as_deref()
        .filter(|s| !s.is_empty());

    sqlx::query(
        "UPDATE photos SET encrypted_blob_id = ?, encrypted_thumb_blob_id = ? WHERE id = ? AND user_id = ?",
    )
    .bind(&req.blob_id)
    .bind(effective_thumb)
    .bind(&photo_id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await?;

    tracing::info!(
        photo_id = %photo_id,
        blob_id = %req.blob_id,
        thumb_blob_id = effective_thumb.unwrap_or("none"),
        user_id = %auth.user_id,
        "Photo marked as encrypted"
    );

    Ok(Json(serde_json::json!({ "ok": true })))
}