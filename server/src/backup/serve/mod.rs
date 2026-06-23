//! Server‐to‐server backup “serving” API.
//!
//! When this instance is in backup mode, other Simple Photos servers can
//! pull data from it via these endpoints. All requests are authenticated
//! with an `X-API-Key` header (validated against `config.backup.api_key`).
//!
//! Endpoints:
//! - `GET  /api/backup/list`                    — list all photos (with IDs)
//! - `GET  /api/backup/list-trash`              — list all trash items (with IDs)
//! - `GET  /api/backup/download/:id`            — download original file
//! - `GET  /api/backup/download/:id/thumb`      — download thumbnail
//! - `POST /api/backup/receive`                 — receive a photo pushed
//!   from the primary server (verifies `X-Content-Hash` if present)
//!
//! This module is split into focused submodules:
//! - [`read`]      — read-only listing + download endpoints.
//! - [`deletions`] — deletion-sync endpoint.
//! - [`galleries`] — secure-gallery sync + retroactive purge.
//! - [`blobs`]     — client-encrypted blob receive endpoint.
//! - [`metadata`]  — full-state metadata table sync.

mod blobs;
mod deletions;
mod galleries;
mod metadata;
mod read;

pub use blobs::backup_receive_blob;
pub use deletions::backup_sync_deletions;
pub use galleries::backup_sync_secure_galleries;
pub use metadata::backup_sync_metadata;
pub use read::{
    backup_download_photo, backup_download_thumb, backup_list_blobs, backup_list_photos,
    backup_list_trash,
};

use axum::http::HeaderMap;

use crate::error::AppError;
use crate::state::AppState;

// ── API-Key Validation ───────────────────────────────────────────────────────

/// Validate the `X-API-Key` header against the configured backup API key.
///
/// Priority:
/// 1. `config.backup.api_key` (TOML / env var) — static, fastest path
/// 2. `server_settings.backup_api_key` (DB) — auto-generated during pairing
///    or when "backup mode" is enabled via the admin UI
///
/// Returns an error if the key is missing, wrong, or backup serving is disabled.
pub(super) async fn validate_api_key(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    // Resolve the expected key: prefer static config, fall back to DB
    let configured_key: String = if let Some(k) = state
        .config
        .backup
        .api_key
        .as_deref()
        .filter(|k| !k.is_empty())
    {
        k.to_string()
    } else {
        // Check DB for a key generated via pairing or admin UI
        let db_key: Option<String> =
            sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_api_key'")
                .fetch_optional(&state.read_pool)
                .await
                .unwrap_or(None);

        db_key.filter(|k| !k.is_empty()).ok_or_else(|| {
            AppError::Forbidden("Backup serving is not enabled on this server".into())
        })?
    };

    let provided_key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing X-API-Key header".into()))?;

    // Constant-time comparison — a plain `!=` short-circuits on the first
    // differing byte and leaks the shared key's length/prefix through timing.
    use subtle::ConstantTimeEq;
    let matches: bool = provided_key
        .as_bytes()
        .ct_eq(configured_key.as_bytes())
        .into();
    if !matches {
        return Err(AppError::Unauthorized("Invalid API key".into()));
    }

    Ok(())
}
