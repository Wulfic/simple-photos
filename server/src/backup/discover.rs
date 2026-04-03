//! LAN server discovery — probing and scanning logic.
//!
//! Discovers Simple Photos servers on the local network via UDP broadcast,
//! localhost/Docker probes, and LAN subnet scanning.

use axum::extract::State;
use axum::Json;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::broadcast;
use super::models::*;

/// GET /api/admin/backup/discover
/// Discover Simple Photos servers on the local network via UDP broadcast.
/// Backup-mode servers broadcast their presence and respond to discovery probes.
///
/// Three discovery phases, each with bounded timeouts:
/// 1. **UDP broadcast** (3s) — listens for backup-mode server beacons.
/// 2. **Localhost/Docker probes** (~2s) — probes common ports on loopback,
///    `host.docker.internal`, and Docker gateway for co-located instances.
/// 3. **LAN subnet scan** (≤10s, only real subnets) — probes fewer ports
///    on the server's actual subnet. Skips fallback subnets to avoid
///    multi-minute timeouts on large/unreachable networks.
///
/// Total worst-case: ~15s. Previous implementation could take 2+ minutes
/// due to probing fallback subnets (192.168.0, 192.168.1, 10.0.0) where
/// unreachable hosts each consumed a 2s TCP timeout.
pub async fn discover_servers(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<DiscoverResponse>, AppError> {
    require_admin(&state, &auth).await?;

    // Overall safety timeout — each phase has its own bounded timeout,
    // so this outer limit is just a crash-stop. Inner phases use streaming
    // collection so partial results survive phase-level timeouts.
    let discover_future = discover_servers_inner(&state);
    let discovered =
        match tokio::time::timeout(std::time::Duration::from_secs(15), discover_future).await {
            Ok(servers) => servers,
            Err(_) => {
                tracing::warn!("Server discovery timed out after 15s");
                Vec::new()
            }
        };

    Ok(Json(DiscoverResponse {
        servers: discovered,
    }))
}

/// Inner discovery logic, separated so we can wrap it in a timeout.
///
/// Uses 3 phases with the dedicated discovery port (default 3301):
///  0. **UDP broadcast** (3s) — instant for backup-mode servers on the same L2.
///  1. **Localhost/Docker probes** — hits discovery port + fallback `/api/discover/info`
///     on loopback and Docker-internal hosts.
///  2. **LAN subnet scan** — probes ONLY the discovery port on each /24 IP,
///     reducing total probes from ~1,500 to ~254. The response includes the
///     server's actual HTTP port, so no multi-port guessing is needed.
async fn discover_servers_inner(state: &AppState) -> Vec<DiscoveredServer> {
    let discovery_port = state.config.server.discovery_port;
    let our_port = state.config.server.port;

    // Pre-seed with our own addresses so we never list ourselves
    let mut existing_addrs = std::collections::HashSet::new();
    existing_addrs.insert(format!("127.0.0.1:{}", our_port));
    existing_addrs.insert(format!("localhost:{}", our_port));
    if let Ok(url) = reqwest::Url::parse(&state.config.server.base_url) {
        if let Some(host) = url.host_str() {
            let port = url.port().unwrap_or(our_port);
            existing_addrs.insert(format!("{}:{}", host, port));
        }
    }
    // Add ALL local interface IPs (Docker bridge, VPN, etc.)
    for ip in broadcast::get_all_local_ips() {
        existing_addrs.insert(format!("{}:{}", ip, our_port));
    }

    // ── Phase 0: UDP broadcast discovery (1s) ────────────────────────────
    let broadcast_results = tokio::task::spawn_blocking(|| {
        broadcast::discover_via_broadcast(std::time::Duration::from_secs(1))
    })
    .await
    .unwrap_or_default();

    // Broadcast is sent exclusively by backup-mode servers (SPBK prefix).
    let mut discovered: Vec<DiscoveredServer> = broadcast_results
        .into_iter()
        .filter(|b| !existing_addrs.contains(&b.address))
        .map(|b| DiscoveredServer {
            address: b.address,
            name: b.name,
            version: b.version,
            mode: Some("backup".to_string()),
            api_key: None,
        })
        .collect();

    // Add broadcast results to dedup set
    for d in &discovered {
        existing_addrs.insert(d.address.clone());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_secs(1))
        .build()
        .unwrap_or_default();

    // ── Phase 1: Probe local/Docker hosts ────────────────────────────────
    // On localhost we probe both the discovery port AND the old-style
    // /api/discover/info on common ports, because Docker containers may
    // not have port 3301 mapped.
    let mut probe_hosts: Vec<String> = vec![
        "127.0.0.1".to_string(),
        "host.docker.internal".to_string(),
        "172.17.0.1".to_string(),
    ];
    if let Some(gw) = crate::backup::broadcast::get_default_gateway() {
        if !probe_hosts.contains(&gw) {
            probe_hosts.push(gw);
        }
    }

    // Build flat list of local probes: (host, port, is_discovery)
    let mut local_probes: Vec<(String, u16, bool)> = Vec::new();

    // First: probe discovery port on all local hosts (fast & definitive)
    if discovery_port != 0 {
        for host in &probe_hosts {
            local_probes.push((host.clone(), discovery_port, true));
        }
    }

    // Fallback: probe /api/discover/info on common ports for Docker containers
    // that might not have the discovery port forwarded.
    // When discovery_port is active, only probe fallback ports on 127.0.0.1
    // to avoid slow timeouts on unreachable Docker IPs.
    let mut local_ports: Vec<u16> = Vec::new();
    let base = (our_port / 10) * 10;
    for p in base..=(base + 9) {
        if p != our_port && p != discovery_port {
            local_ports.push(p);
        }
    }
    for &p in &[3000u16, 3001, 3002, 3003, 8080, 8081, 8082, 8083, 8443] {
        if p != our_port && p != discovery_port && !local_ports.contains(&p) {
            local_ports.push(p);
        }
    }
    for &port in &local_ports {
        for host in &probe_hosts {
            // When discovery_port is set, skip fallback-port probes on
            // non-loopback hosts — they'll be found via the discovery
            // port in Phase 2 and probing unreachable Docker IPs is slow.
            if discovery_port != 0
                && host != "127.0.0.1"
                && host != "host.docker.internal"
                && !host.starts_with("172.")
                && !host.starts_with("192.")
            {
                continue;
            }
            let addr = format!("{}:{}", host, port);
            if existing_addrs.contains(&addr) {
                continue;
            }
            local_probes.push((host.clone(), port, false));
        }
    }

    let mut local_futures = Vec::new();
    for (host_owned, port, is_discovery) in local_probes {
        let c = client.clone();
        local_futures.push(async move {
            if is_discovery {
                probe_discovery_port(&c, &host_owned, port).await
            } else {
                probe_server(&c, &host_owned, port).await
            }
        });
    }

    let local_results = futures_util::future::join_all(local_futures).await;
    for result in local_results.into_iter().flatten() {
        if !existing_addrs.contains(&result.address) {
            existing_addrs.insert(result.address.clone());
            discovered.push(result);
        }
    }

    tracing::info!(
        "[DISCOVER] Phase 1 complete: {} servers found so far",
        discovered.len()
    );

    // ── Phase 2: LAN subnet scan via discovery port only ─────────────────
    let mut subnets: Vec<String> = Vec::new();
    if let Ok(url) = reqwest::Url::parse(&state.config.server.base_url) {
        if let Some(host) = url.host_str() {
            let parts: Vec<&str> = host.split('.').collect();
            if parts.len() == 4 {
                let subnet = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
                subnets.push(subnet);
            }
        }
    }
    if let Some(local_ip) = broadcast::get_local_ip() {
        let parts: Vec<&str> = local_ip.split('.').collect();
        if parts.len() == 4 {
            let subnet = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
            if !subnets.contains(&subnet) {
                subnets.push(subnet);
            }
        }
    }
    if let Ok(addrs) = tokio::net::lookup_host("host.docker.internal:0").await {
        for addr in addrs {
            let ip = addr.ip().to_string();
            let parts: Vec<&str> = ip.split('.').collect();
            if parts.len() == 4 {
                let subnet = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
                if !subnets.contains(&subnet) {
                    subnets.push(subnet);
                }
            }
        }
    }

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(200));

    // Build a flat list of (ip, port, use_discovery_protocol) probes
    let mut probes: Vec<(String, u16, bool)> = Vec::new();
    for subnet in &subnets {
        for host_id in 1..=254u8 {
            let ip = format!("{}.{}", subnet, host_id);
            if discovery_port != 0 {
                let addr = format!("{}:{}", ip, discovery_port);
                if !existing_addrs.contains(&addr) {
                    probes.push((ip, discovery_port, true));
                }
            } else {
                for &port in &[our_port, 3000u16, 8080, 8081, 8082, 8083] {
                    let addr = format!("{}:{}", ip, port);
                    if !existing_addrs.contains(&addr) {
                        probes.push((ip.clone(), port, false));
                    }
                }
            }
        }
    }

    let mut lan_futures = Vec::new();
    for (ip, port, is_discovery) in probes {
        let c = client.clone();
        let permit = sem.clone();
        lan_futures.push(async move {
            let _permit = permit.acquire().await.ok()?;
            if is_discovery {
                probe_discovery_port(&c, &ip, port).await
            } else {
                probe_health_only(&c, &format!("{}:{}", ip, port)).await
            }
        });
    }

    // Use FuturesUnordered + streaming so partial results are preserved
    // if the deadline fires (join_all + unwrap_or_default discards everything).
    let mut stream: futures_util::stream::FuturesUnordered<_> = lan_futures.into_iter().collect();

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match tokio::time::timeout_at(deadline, futures_util::StreamExt::next(&mut stream)).await {
            Ok(Some(Some(result))) => {
                if !existing_addrs.contains(&result.address) {
                    existing_addrs.insert(result.address.clone());
                    discovered.push(result);
                }
            }
            Ok(Some(None)) => { /* probe returned None */ }
            Ok(None) => break, // stream exhausted
            Err(_) => {
                tracing::warn!(
                    "[DISCOVER] Phase 2 subnet scan timed out after 10s with {} servers",
                    discovered.len()
                );
                break;
            }
        }
    }

    // ── Deduplication: prefer routable addresses over loopback/Docker ────
    let mut by_port: std::collections::HashMap<u16, DiscoveredServer> =
        std::collections::HashMap::new();
    for server in discovered {
        let port = server
            .address
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(0);
        let entry = by_port.entry(port).or_insert_with(|| server.clone());
        let current_ip = entry.address.split(':').next().unwrap_or("");
        let new_ip = server.address.split(':').next().unwrap_or("");
        let score = |ip: &str| -> u8 {
            if ip == "127.0.0.1" || ip == "localhost" {
                0
            } else if ip.starts_with("172.") || ip == "host.docker.internal" {
                1
            } else {
                2
            }
        };
        if score(new_ip) > score(current_ip) {
            // Prefer routable address, but merge api_key and mode from both
            let api_key = server.api_key.or_else(|| entry.api_key.clone());
            let mode = server.mode.or_else(|| entry.mode.clone());
            *entry = DiscoveredServer {
                api_key,
                mode,
                ..server
            };
        } else {
            // Keep current (better-scored) address — merge any extra info in
            if entry.api_key.is_none() && server.api_key.is_some() {
                entry.api_key = server.api_key;
            }
            if entry.mode.is_none() && server.mode.is_some() {
                entry.mode = server.mode;
            }
        }
    }
    // Filter: primary server scan should only surface backup-mode servers.
    // Servers whose mode is explicitly "primary" are excluded.
    // Servers with mode=None (responded via /health only, mode unknown) are
    // kept for backward compatibility with older server versions.
    let deduped: Vec<DiscoveredServer> = by_port
        .into_values()
        .filter(|s| s.mode.as_deref() != Some("primary"))
        .collect();

    tracing::info!(
        "[DISCOVER] Discovery complete: {} backup servers found",
        deduped.len()
    );

    deduped
}

/// Probe a single host:port for a Simple Photos server.
/// Tries `/api/discover/info` first (returns API key for localhost),
/// then falls back to `/health`.
async fn probe_server(client: &reqwest::Client, host: &str, port: u16) -> Option<DiscoveredServer> {
    let info_url = format!("http://{}:{}/api/discover/info", host, port);

    // Primary probe: /api/discover/info (loopback-only, returns API key)
    if let Ok(resp) = client.get(&info_url).send().await {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if body.get("service").and_then(|s| s.as_str()) == Some("simple-photos") {
                    let name = body
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let version = body
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let api_key = body
                        .get("api_key")
                        .and_then(|k| k.as_str())
                        .filter(|k| !k.is_empty())
                        .map(|k| k.to_string());
                    let mode = body
                        .get("mode")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());
                    return Some(DiscoveredServer {
                        address: format!("{}:{}", host, port),
                        name,
                        version,
                        mode,
                        api_key,
                    });
                }
            }
        }
    }
    // Fallback: /health (works for all servers, any network position)
    probe_health_only(client, &format!("{}:{}", host, port)).await
}

/// Probe a single address via `/health` endpoint only.
async fn probe_health_only(client: &reqwest::Client, addr: &str) -> Option<DiscoveredServer> {
    let url = format!("http://{}/health", addr);
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if body.get("service").and_then(|s| s.as_str()) == Some("simple-photos") {
                    let name = body
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let version = body
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    return Some(DiscoveredServer {
                        address: addr.to_string(),
                        name,
                        version,
                        mode: None, // /health doesn't report mode
                        api_key: None,
                    });
                }
            }
            None
        }
        _ => None,
    }
}

/// Probe the dedicated discovery port on a host.
///
/// The discovery listener returns `{ service, name, version, port, mode, ... }`.
/// We use the `port` field from the response to build the real `address`
/// (host:actual_port) for the returned `DiscoveredServer`.
async fn probe_discovery_port(
    client: &reqwest::Client,
    host: &str,
    discovery_port: u16,
) -> Option<DiscoveredServer> {
    let url = format!("http://{}:{}/", host, discovery_port);
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if body.get("service").and_then(|s| s.as_str()) == Some("simple-photos") {
                    let name = body
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let version = body
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    // The discovery response includes the server's real HTTP port
                    let actual_port = body
                        .get("port")
                        .and_then(|p| p.as_u64())
                        .map(|p| p as u16)
                        .unwrap_or(discovery_port);
                    let mode = body
                        .get("mode")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());
                    return Some(DiscoveredServer {
                        address: format!("{}:{}", host, actual_port),
                        name,
                        version,
                        mode,
                        api_key: None,
                    });
                }
            }
            None
        }
        _ => None,
    }
}
