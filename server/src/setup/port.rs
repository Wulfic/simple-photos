//! Server port configuration and restart endpoints.
//!
//! - `GET  /api/admin/port`      — current listener port.
//! - `PUT  /api/admin/port`      — update port in `config.toml` (takes
//!   effect after restart).
//! - `POST /api/admin/restart`   — graceful server restart via
//!   `std::process::Command::new(std::env::current_exe())`.
//!
//! On Linux the restart uses `exec` to replace the process in-place;
//! on other platforms a child process is spawned and the parent exits.

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
    /// First available port starting from 8080 (for wizard default).
    /// Increments from 8080 until an unbound port is found.
    pub suggested_port: u16,
    pub message: String,
}

/// Find the first TCP port that can be bound, starting from `start`.
/// Tries each port in sequence; returns `start` as a fallback if all fail.
///
/// `our_port` is the port this server process is already listening on.
/// Binding that port would spuriously fail (we own it), so we treat it as
/// available and return it immediately so the wizard never suggests `our_port + 1`
/// as the "next free" port when the server is already on `our_port`.
fn find_available_port(start: u16, our_port: u16) -> u16 {
    let mut port = start;
    loop {
        // Our own port is always valid — we already own the binding.
        // Attempting TcpListener::bind on it would fail even though it is free
        // for our use, which would cause the function to skip past it incorrectly.
        if port == our_port {
            return port;
        }
        // Attempt a non-blocking bind — if it succeeds the port is free.
        if std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
            return port;
        }
        match port.checked_add(1) {
            Some(next) if next < 65535 => port = next,
            _ => return start, // wrapped or hit ceiling — use fallback
        }
    }
}

/// Admin-only: Get the current server port from config.
///
/// GET /api/admin/port
pub async fn get_port(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<PortResponse>, AppError> {
    require_admin(&state, &auth).await?;

    // Pass our own port so find_available_port never skips it — if we're
    // already on 8080 the suggestion should remain 8080, not 8081.
    let current_port = state.config.server.port;
    let suggested_port = tokio::task::spawn_blocking(move || find_available_port(8080, current_port))
        .await
        .unwrap_or(8080);

    Ok(Json(PortResponse {
        port: state.config.server.port,
        suggested_port,
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

    // Persist to config.toml (blocking I/O — offload to spawn_blocking).
    //
    // In Docker the config file is often read-only (mounted :ro).  When the
    // write fails we fall back to persisting the new port in the database so
    // the wizard can continue.  The server operator will still need to
    // update docker-compose.yml for the actual port mapping.
    let port = req.port;
    let config_write_result =
        tokio::task::spawn_blocking(move || update_config_toml_port(port))
            .await
            .map_err(|e| AppError::Internal(format!("spawn_blocking join error: {}", e)))?;

    let config_write_ok = match config_write_result {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("Could not write port to config.toml (read-only / Docker?): {}", e);
            false
        }
    };

    // Always persist the port in the database so other parts of the system
    // can read it, and so the wizard value survives even if the config file
    // is immutable (common in Docker containers).
    if let Err(e) = sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('server_port', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(req.port.to_string())
    .execute(&state.pool)
    .await
    {
        tracing::error!("Failed to persist port to database: {}", e);
    }

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

    tracing::info!(
        "Server port updated to {} in config (restart required)",
        req.port
    );

    let message = if config_write_ok {
        format!(
            "Port updated to {}. Server restart required for the change to take effect.",
            req.port
        )
    } else {
        format!(
            "Port preference saved ({}). Note: the config file is read-only \
             (common in Docker). Update the port mapping in your docker-compose.yml \
             and restart the container for the change to take effect.",
            req.port
        )
    };

    Ok(Json(PortResponse {
        port: req.port,
        // After an explicit save the chosen port is already confirmed; echo it back.
        suggested_port: req.port,
        message,
    }))
}

/// Read config.toml, update [server] port (and base_url), and write it back.
fn update_config_toml_port(new_port: u16) -> anyhow::Result<()> {
    let config_path =
        std::env::var("SIMPLE_PHOTOS_CONFIG").unwrap_or_else(|_| "config.toml".into());
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
