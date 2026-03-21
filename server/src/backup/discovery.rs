//! Dedicated LAN discovery listener.
//!
//! Runs a lightweight HTTP server on a well-known port (default 3301) so
//! that clients — other servers, the web UI, or mobile apps — can discover
//! Simple Photos instances by probing a single port per IP instead of
//! scanning dozens of common ports.
//!
//! The listener exposes one endpoint:
//!
//! ```text
//! GET /  →  { "service": "simple-photos", "name": "…", "version": "…",
//!              "port": 8080, "mode": "primary", "api_key_required": false }
//! ```
//!
//! Clients read the `port` field to learn the server's actual HTTP port,
//! then connect to it for all real API calls.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::Json;
use axum::Router;
use sqlx::SqlitePool;
use tower_http::cors::{Any, CorsLayer};

use crate::config::AppConfig;

/// Well-known discovery port. Matches the default in `ServerConfig`.
#[allow(dead_code)]
pub const DEFAULT_DISCOVERY_PORT: u16 = 3301;

/// Shared state for the discovery micro-service (intentionally minimal).
#[derive(Clone)]
struct DiscoveryState {
    pool: SqlitePool,
    config: Arc<AppConfig>,
}

/// JSON response returned by the discovery endpoint.
#[derive(serde::Serialize)]
struct DiscoveryResponse {
    /// Always `"simple-photos"` — used by clients to confirm the service identity.
    service: &'static str,
    /// Human-readable server name (from `server_settings` or a default).
    name: String,
    /// Server version string (compiled from `Cargo.toml`).
    version: &'static str,
    /// The actual HTTP(S) port the server API is running on.
    port: u16,
    /// Operating mode: `"primary"` or `"backup"`.
    mode: String,
    /// Whether an API key is required to access backup endpoints.
    api_key_required: bool,
}

/// Start the discovery listener on the configured port.
///
/// This is designed to be spawned as a background task from `main()`.
/// It will log a warning and return (without crashing the server) if the
/// port is already in use.
pub async fn run_discovery_listener(
    pool: SqlitePool,
    config: Arc<AppConfig>,
) {
    let discovery_port = config.server.discovery_port;
    if discovery_port == 0 {
        tracing::info!("Discovery listener disabled (discovery_port = 0)");
        return;
    }

    // Don't bind the discovery port if it's the same as the main server port
    if discovery_port == config.server.port {
        tracing::warn!(
            "Discovery port {} is the same as the server port — skipping dedicated listener \
             (discovery is still available via /api/discover/info on the main port)",
            discovery_port
        );
        return;
    }

    let state = DiscoveryState {
        pool,
        config: config.clone(),
    };

    let app = Router::new()
        .route("/", get(discovery_handler))
        // Wide-open CORS — this is a read-only, unauthenticated discovery endpoint
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.server.host, discovery_port)
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], discovery_port)));

    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            tracing::info!(
                "Discovery listener ready on port {} (server API on port {})",
                discovery_port,
                config.server.port
            );
            if let Err(e) = axum::serve(listener, app.into_make_service()).await {
                tracing::error!("Discovery listener exited with error: {}", e);
            }
        }
        Err(e) => {
            tracing::warn!(
                "Could not bind discovery port {}: {} — discovery will rely on \
                 UDP broadcast and /api/discover/info fallback",
                discovery_port,
                e
            );
        }
    }
}

/// `GET /` — return server identity and connection info.
///
/// No authentication required. The response is intentionally minimal:
/// it reveals the server name, version, and port but no sensitive data.
/// API keys are never exposed here; the `api_key_required` flag only
/// indicates *whether* one is needed, not what it is.
async fn discovery_handler(
    State(state): State<DiscoveryState>,
) -> Json<DiscoveryResponse> {
    // Fetch server name from DB (fallback to sensible default)
    let name: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'server_name'",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "Simple Photos".to_string());

    // Fetch operating mode (primary / backup)
    let mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'backup_mode'",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "primary".to_string());

    // Check whether an API key is configured (don't reveal the key itself)
    let api_key_required: bool = state
        .config
        .backup
        .api_key
        .as_deref()
        .map(|k| !k.is_empty())
        .unwrap_or(false)
        || sqlx::query_scalar::<_, Option<String>>(
            "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
        )
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .map(|v| !v.is_empty())
        .unwrap_or(false);

    Json(DiscoveryResponse {
        service: "simple-photos",
        name,
        version: crate::VERSION,
        port: state.config.server.port,
        mode,
        api_key_required,
    })
}
