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
            let hex_str = std::str::from_utf8(chunk)
                .map_err(|_| AppError::BadRequest("Invalid UTF-8 in key_hex".into()))?;
            buf[i] = u8::from_str_radix(hex_str, 16)
                .map_err(|_| AppError::BadRequest("Invalid hex in key_hex".into()))?;
        }
        buf
    };

    // Wrap and store the key
    crate::crypto::store_wrapped_key(&state.pool, &key_bytes, &state.config.auth.jwt_secret)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to store encryption key: {}", e)))?;

    tracing::info!(user_id = %auth.user_id, "Encryption key stored by admin");

    // Trigger a full scan → encrypt cycle.  During first-run setup the photos
    // table is still empty (the startup autoscan ran before the admin existed),
    // so we must scan *first*, then encrypt any newly discovered files.
    //
    // The scan runs synchronously so the frontend can navigate to the gallery
    // immediately after the response and find the discovered photos.
    // Encryption is spawned in the background because it can take a while.
    {
        let storage_root = (**state.storage_root.load()).clone();
        let count = if let Ok(_guard) = state.scan_lock.try_lock() {
            crate::backup::autoscan::run_auto_scan_public(&state.pool, &storage_root).await
        } else {
            tracing::info!("[STORE_KEY] Scan skipped — another scan is in progress");
            0
        };
        if count > 0 {
            tracing::info!("[STORE_KEY] Discovered {} new files, starting encryption", count);
        }
        // Phase 2: encrypt any unencrypted photos in the background
        let pool_clone = state.pool.clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            crate::photos::server_migrate::auto_migrate_after_scan(
                pool_clone,
                storage_root,
                jwt_secret,
            )
            .await;
        });
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}
