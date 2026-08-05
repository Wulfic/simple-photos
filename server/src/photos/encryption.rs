//! Encryption key storage endpoint.
//!
//! The server always operates in encrypted mode (AES-256-GCM, client-side).
//! This module handles persisting the client-derived encryption key so
//! server-side operations (autoscan) can process photos.
//!
//! - `POST /api/admin/encryption/store-key`    — persist the encryption key
//! - `POST /api/admin/encryption/retry-parked` — un-park abandoned photos

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

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
        .map_err(|e| AppError::Internal(format!("Failed to store encryption key: {e}")))?;

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
            tracing::info!(
                "[STORE_KEY] Discovered {} new files, starting encryption",
                count
            );
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

// ── Retry parked photos ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RetryParkedResponse {
    /// How many parked photos were re-admitted to the encryption queue.
    pub cleared: u64,
}

/// POST /api/admin/encryption/retry-parked
///
/// Clears the parked marker on every photo the encryption migration gave up on,
/// so the next pass retries them with a full attempt budget.
///
/// This is the missing half of the three-strike cap. The conversion cap (#40)
/// has two escape hatches — the file changing on disk, and
/// `/admin/conversion/retry-failed` — because a file can be retired by a
/// *server-side* defect that a later server fixes. The encryption cap had
/// neither, and its failure mode is strictly worse: a parked photo keeps its
/// **plaintext original at rest**. The pre-`SPCHNKB2` whole-file encrypt needed
/// ~5x RAM and OOM-aborted on large videos; every file it killed was parked
/// permanently, and stayed unencrypted long after the chunked path landed.
///
/// Kicks the migration on success rather than waiting for the next autoscan, so
/// the operator sees the count move instead of wondering whether the button did
/// anything. Idempotent: with nothing parked it reports `cleared: 0` and starts
/// no work.
pub async fn retry_parked_encryption(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<RetryParkedResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let cleared = crate::photos::server_migrate::retry_parked(&state.pool, &auth.user_id)
        .await
        .map_err(|e| {
            tracing::error!(
                user_id = %auth.user_id,
                error = %e,
                "[SERVER_MIG] failed to un-park photos for retry"
            );
            AppError::Internal("failed to un-park photos".to_string())
        })?;

    tracing::info!(
        user_id = %auth.user_id,
        cleared,
        "[SERVER_MIG] Admin re-admitted parked photos to the encryption queue"
    );

    crate::audit::log_background(
        &state.pool,
        crate::audit::AuditEvent::AdminAction,
        Some(serde_json::json!({
            "action": "encryption_retry_parked",
            "cleared": cleared,
        })),
    );

    // Nothing was parked — starting a migration would only log a no-op pass.
    if cleared > 0 {
        let pool = state.pool.clone();
        let storage_root = (**state.storage_root.load()).clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            crate::photos::server_migrate::auto_migrate_after_scan(pool, storage_root, jwt_secret)
                .await;
        });
    }

    Ok(Json(RetryParkedResponse { cleared }))
}
