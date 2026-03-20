//! Health check handler and local-discovery info endpoint.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::state::AppState;

/// GET /health — lightweight health check for load balancers and uptime monitors.
pub async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "simple-photos",
        "version": crate::VERSION
    }))
}

/// GET /api/discover/info
///
/// Loopback-only endpoint used by the primary server's `discover_servers`
/// handler to retrieve the backup mode and API key of a co-located backup
/// server (e.g. a Docker container mapped to a localhost port).
///
/// Only responds to requests originating from 127.0.0.1 or ::1 — all others
/// receive 403 Forbidden.  No authentication token is required because
/// loopback access implies the caller is a process on the same machine.
pub async fn discover_info(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    // Strictly loopback-only — reject anything that looks external.
    if !peer.ip().is_loopback() {
        return Err(AppError::Forbidden(
            "discover/info is only accessible from localhost".into(),
        ));
    }

    // Fetch the current backup mode (default: "primary").
    let mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'backup_mode'",
    )
    .fetch_optional(&state.read_pool)
    .await?
    .unwrap_or_else(|| "primary".to_string());

    // Only expose the API key when this server is operating as a backup.
    let api_key: Option<String> = if mode == "backup" {
        // Config-file key takes priority over the DB-stored key.
        if let Some(key) = state
            .config
            .backup
            .api_key
            .as_deref()
            .filter(|k| !k.is_empty())
        {
            Some(key.to_string())
        } else {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
            )
            .fetch_optional(&state.read_pool)
            .await?
            .flatten()
        }
    } else {
        None
    };

    Ok(Json(json!({
        "service": "simple-photos",
        "version": crate::VERSION,
        "mode": mode,
        "api_key": api_key,
    })))
}
