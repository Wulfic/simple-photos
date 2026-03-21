//! Encryption key storage endpoint.
//!
//! The server always operates in encrypted mode (AES-256-GCM, client-side).
//! This module handles persisting the client-derived encryption key so
//! server-side operations (autoscan) can process photos.
//!
//! - `POST /api/admin/encryption/store-key`   — persist the encryption key

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

// ── Store encryption key ────────────────────────────────────────────────────

/// POST /api/admin/encryption/store-key
/// Persists the client-derived AES-256 encryption key (wrapped with the
/// server's JWT secret) so that server-side operations (autoscan)
/// can process photos autonomously.
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
