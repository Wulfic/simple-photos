//! Health check handler and local-discovery info endpoint.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::state::AppState;

/// GET /health — lightweight health check for load balancers and uptime monitors.
///
/// Reports `"ok"` when all subsystems are healthy, `"degraded"` when the
/// storage backend is unreachable (network drive disconnected, mount stale,
/// etc.).  The `storage` field provides the detailed storage status.
pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let storage_ok = state.is_storage_available();
    let status = if storage_ok { "ok" } else { "degraded" };
    Json(json!({
        "status": status,
        "service": "simple-photos",
        "version": crate::VERSION,
        "storage": if storage_ok { "connected" } else { "disconnected" }
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
    // Allow loopback and Docker-internal networks (172.16.0.0/12).
    // When the primary server probes a Docker container via a port-mapped
    // localhost port, Docker NAT rewrites the source address to the bridge
    // gateway (e.g. 172.17.0.1).  The strict loopback-only check rejected
    // these legitimate intra-host requests, causing discovery to miss all
    // Docker backup containers.
    let ip = peer.ip();
    let is_local = ip.is_loopback() || is_docker_internal(ip);
    if !is_local {
        return Err(AppError::Forbidden(
            "discover/info is only accessible from localhost or Docker networks".into(),
        ));
    }

    // Fetch the current backup mode (default: "primary").
    let mode: String =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_mode'")
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

    // Include a human-readable name for the discovery UI.
    let name: String =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'server_name'")
            .fetch_optional(&state.read_pool)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                if mode == "backup" {
                    "Simple Photos Backup".to_string()
                } else {
                    "Simple Photos".to_string()
                }
            });

    // Extract host:port from base_url so callers use the externally-reachable
    // address (Docker containers report internal ports that differ from the
    // host-mapped ports).
    let address = reqwest::Url::parse(&state.config.server.base_url)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_string();
            let port = url.port().unwrap_or(state.config.server.port);
            Some(format!("{}:{}", host, port))
        });

    Ok(Json(json!({
        "service": "simple-photos",
        "name": name,
        "version": crate::VERSION,
        "mode": mode,
        "api_key": api_key,
        "address": address,
    })))
}

/// Returns `true` if the IP belongs to a Docker-internal network (172.16.0.0/12)
/// or other common private bridge ranges used by container runtimes.
fn is_docker_internal(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 172.16.0.0/12 — Docker default bridge and custom networks
            (octets[0] == 172 && (16..=31).contains(&octets[1]))
            // 10.0.0.0/8 — some container runtimes use this range
            || octets[0] == 10
        }
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    }
}
