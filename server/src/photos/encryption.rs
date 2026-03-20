//! Encryption settings endpoint.
//!
//! The server always operates in encrypted mode (AES-256-GCM, client-side).
//! This module exposes the read-only settings endpoint, the
//! `mark-encrypted` helper used by the auto-migration on startup, and
//! a `store-key` endpoint for persisting the wrapped encryption key.
//!
//! - `GET  /api/settings/encryption`          — confirms encrypted mode
//! - `POST /api/admin/encryption/store-key`   — persist the encryption key
//! - `POST /api/photos/:id/mark-encrypted`    — link a plain photo to its
//!   encrypted blob so it won't be re-migrated.

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::EncryptionSettingsResponse;

/// GET /api/settings/encryption
/// Always returns `"encrypted"` — the server does not support plain mode.
pub async fn get_encryption_settings(
    State(_state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<EncryptionSettingsResponse>, AppError> {
    Ok(Json(EncryptionSettingsResponse {
        encryption_mode: "encrypted".to_string(),
    }))
}

// ── Store encryption key ────────────────────────────────────────────────────

/// POST /api/admin/encryption/store-key
/// Persists the client-derived AES-256 encryption key (wrapped with the
/// server's JWT secret) so that server-side operations (autoscan, auto-
/// migration) can encrypt photos autonomously.
///
/// Idempotent — safe to call on every login.
#[derive(Debug, Deserialize)]
pub struct StoreKeyRequest {
    pub key_hex: String,
}

pub async fn store_encryption_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<StoreKeyRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    // Validate the key is a 64-char hex string (32 bytes)
    if req.key_hex.len() != 64 || !req.key_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest(
            "key_hex must be a 64-character hex string (32 bytes)".into(),
        ));
    }

    // Decode hex → 32-byte key
    let key_bytes: [u8; 32] = {
        let mut buf = [0u8; 32];
        for (i, chunk) in req.key_hex.as_bytes().chunks(2).enumerate() {
            buf[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16)
                .map_err(|_| AppError::BadRequest("Invalid hex in key_hex".into()))?;
        }
        buf
    };

    // Wrap and store the key
    crate::crypto::store_wrapped_key(&state.pool, &key_bytes, &state.config.auth.jwt_secret)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to store encryption key: {}", e)))?;

    tracing::info!(user_id = %auth.user_id, "Encryption key stored by admin");

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// POST /api/photos/{id}/mark-encrypted
/// Link a plain photo to its encrypted blob so it won't be re-migrated.
/// Used by the auto-migration on startup for existing plain-mode photos.
/// Also accepts an optional `thumb_blob_id` so both fields can be set
/// in a single request.
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