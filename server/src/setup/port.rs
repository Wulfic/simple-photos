//! Server port configuration and restart endpoints.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::admin::require_admin;

// ── Port configuration ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PortResponse {
    pub port: u16,
    pub message: String,
}

/// Admin-only: Get the current server port from config.
///
/// GET /api/admin/port
pub async fn get_port(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<PortResponse>, AppError> {
    require_admin(&state, &auth).await?;

    Ok(Json(PortResponse {
        port: state.config.server.port,
        message: "Current server port".into(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePortRequest {
    pub port: u16,
}

/// Admin-only: Update the server port in config.toml.
///
/// PUT /api/admin/port
///
/// This only persists the change to config.toml. The server must be
/// restarted for the new port to take effect.
pub async fn update_port(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<UpdatePortRequest>,
) -> Result<Json<PortResponse>, AppError> {
    require_admin(&state, &auth).await?;

    // Validate port range (1024–65535 for non-privileged ports)
    if req.port < 1024 {
        return Err(AppError::BadRequest(
            "Port must be 1024 or higher (non-privileged range)".into(),
        ));
    }

    // Persist to config.toml
    update_config_toml_port(req.port).map_err(|e| {
        tracing::error!("Failed to update port in config.toml: {}", e);
        AppError::Internal(format!("Failed to save port configuration: {}", e))
    })?;

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "update_port",
            "new_port": req.port,
        })),
    )
    .await;

    tracing::info!("Server port updated to {} in config (restart required)", req.port);

    Ok(Json(PortResponse {
        port: req.port,
        message: format!(
            "Port updated to {}. Server restart required for the change to take effect.",
            req.port
        ),
    }))
}

/// Read config.toml, update [server] port (and base_url), and write it back.
fn update_config_toml_port(new_port: u16) -> anyhow::Result<()> {
    let config_path = std::env::var("SIMPLE_PHOTOS_CONFIG")
        .unwrap_or_else(|_| "config.toml".into());
    let contents = std::fs::read_to_string(&config_path)?;
    let mut doc: toml::Table = contents.parse()?;

    if let Some(server) = doc.get_mut("server").and_then(|v| v.as_table_mut()) {
        server.insert("port".to_string(), toml::Value::Integer(new_port as i64));

        // Also update base_url to reflect the new port.
        if let Some(base_url) = server.get("base_url").and_then(|v| v.as_str()) {
            let updated = if let Some(colon_pos) = base_url.rfind(':') {
                let after_colon = &base_url[colon_pos + 1..];
                let port_end = after_colon.find('/').unwrap_or(after_colon.len());
                if after_colon[..port_end].parse::<u16>().is_ok() {
                    format!(
                        "{}:{}{}",
                        &base_url[..colon_pos],
                        new_port,
                        &after_colon[port_end..]
                    )
                } else {
                    base_url.to_string()
                }
            } else {
                base_url.to_string()
            };
            server.insert("base_url".to_string(), toml::Value::String(updated));
        }
    }

    std::fs::write(&config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

// ── Server restart ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RestartResponse {
    pub message: String,
}

/// Admin-only: Trigger a graceful server restart.
///
/// POST /api/admin/restart
///
/// The server exits after a short delay (to allow the HTTP response to be sent).
/// A service manager (systemd, Docker, etc.) or the user is expected to restart
/// the process, which will pick up any config.toml changes (e.g. new port).
pub async fn restart_server(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
) -> Result<Json<RestartResponse>, AppError> {
    require_admin(&state, &auth).await?;

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({ "action": "server_restart" })),
    )
    .await;

    tracing::info!("Server restart requested by admin — shutting down in 1 second");

    // Spawn a task that exits after a brief delay so the HTTP response is sent first
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        tracing::info!("Exiting for restart…");
        std::process::exit(0);
    });

    Ok(Json(RestartResponse {
        message: "Server is restarting. Please wait…".into(),
    }))
}
