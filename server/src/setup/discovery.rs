//! LAN server discovery for the first-run setup wizard.
//!
//! The `discover()` handler orchestrates three phases (UDP broadcast, local
//! probing, subnet scan) implemented in [`super::discovery_phases`].

use axum::extract::State;
use axum::Json;

use crate::error::AppError;
use crate::state::AppState;

use super::discovery_phases::{
    deduplicate_servers, phase1_udp_broadcast, phase2_local_probing, phase3_subnet_scan,
};

// ── Handler ─────────────────────────────────────────────────────────────────

/// GET /api/setup/discover
///
/// Discover Simple Photos servers on the local network.
///
/// Uses the dedicated discovery port (default 3301) to find servers with a
/// single probe per IP, supplemented by UDP broadcast and localhost fallbacks.
///
/// Only works during first-run setup (zero users in DB) — no auth required.
pub async fn discover(State(state): State<AppState>) -> Result<Json<serde_json::Value>, AppError> {
    // Guard: only works when no users exist
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.read_pool)
        .await?;

    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed.".into(),
        ));
    }

    let our_port = state.config.server.port;
    let discovery_port = state.config.server.discovery_port;
    let mut discovered: Vec<serde_json::Value> = Vec::new();
    let mut existing_addrs = std::collections::HashSet::new();

    // Pre-seed existing_addrs with our own addresses so we never list ourselves
    existing_addrs.insert(format!("127.0.0.1:{}", our_port));
    existing_addrs.insert(format!("localhost:{}", our_port));
    if let Ok(url) = reqwest::Url::parse(&state.config.server.base_url) {
        if let Some(host) = url.host_str() {
            let port = url.port().unwrap_or(our_port);
            existing_addrs.insert(format!("{}:{}", host, port));
        }
    }
    if let Some(local_ip) = crate::backup::broadcast::get_local_ip() {
        existing_addrs.insert(format!("{}:{}", local_ip, our_port));
    }
    for ip in crate::backup::broadcast::get_all_local_ips() {
        existing_addrs.insert(format!("{}:{}", ip, our_port));
    }

    // HTTP client with tight timeouts for network probes
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_secs(1))
        .build()
        .unwrap_or_default();

    phase1_udp_broadcast(&mut discovered, &mut existing_addrs).await;

    phase2_local_probing(
        &client, discovery_port, our_port,
        &mut discovered, &mut existing_addrs,
    ).await;

    phase3_subnet_scan(
        &client, &state.config.server.base_url, discovery_port, our_port,
        &mut discovered, &mut existing_addrs,
    ).await;

    let discovered_len = discovered.len();
    let final_servers = deduplicate_servers(discovered);
    tracing::info!(
        "Discovery: found {} servers ({} after dedup, {} after primary-only filter)",
        discovered_len,
        final_servers.len(),
        final_servers.len()
    );

    Ok(Json(serde_json::json!({ "servers": final_servers })))
}
