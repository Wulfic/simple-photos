//! Discovery phases and helpers extracted from `discovery.rs`.
//!
//! Contains parsing helpers, deduplication, probe logic, and the three
//! discovery phases (UDP broadcast, local probing, subnet scan).

// ── Parsing helpers ─────────────────────────────────────────────────────────

/// Parse a response from the dedicated discovery port (`GET /`).
///
/// Discovery-port responses include an `address` field with the externally-
/// reachable `host:port` (from `base_url`). When present, this is preferred
/// over constructing an address from the probe IP + the reported `port`,
/// since Docker containers report their internal port which differs from the
/// host-mapped port.
fn parse_discovery_response(
    body: &serde_json::Value,
    ip: &str,
    port: u16,
) -> Option<serde_json::Value> {
    if body.get("service").and_then(|s| s.as_str()) != Some("simple-photos") {
        return None;
    }
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
    // Prefer the explicit address from the response (base_url-derived),
    // falling back to probe_ip:reported_port for backward compatibility.
    let address = body
        .get("address")
        .and_then(|a| a.as_str())
        .map(|a| a.to_string())
        .unwrap_or_else(|| {
            let actual_port = body
                .get("port")
                .and_then(|p| p.as_u64())
                .map(|p| p as u16)
                .unwrap_or(port);
            format!("{}:{}", ip, actual_port)
        });
    let mode = body
        .get("mode")
        .and_then(|m| m.as_str())
        .unwrap_or("primary")
        .to_string();
    Some(serde_json::json!({
        "address": address,
        "name": name,
        "version": version,
        "mode": mode,
    }))
}

/// Parse a response from `/api/discover/info` or `/health`.
///
/// Prefers the `address` field (externally-reachable `host:port` derived from
/// `base_url`) when present. Falls back to `probe_ip:probe_port` for servers
/// that don't include it (e.g. older versions or plain `/health`).
fn parse_server_response(
    body: &serde_json::Value,
    ip: &str,
    port: u16,
    default_name: &str,
) -> Option<serde_json::Value> {
    if body.get("service").and_then(|s| s.as_str()) != Some("simple-photos") {
        return None;
    }
    let name = body
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or(default_name)
        .to_string();
    let version = body
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let mode = body
        .get("mode")
        .and_then(|m| m.as_str())
        .unwrap_or("primary")
        .to_string();
    let address = body
        .get("address")
        .and_then(|a| a.as_str())
        .map(|a| a.to_string())
        .unwrap_or_else(|| format!("{}:{}", ip, port));
    Some(serde_json::json!({
        "address": address,
        "name": name,
        "version": version,
        "mode": mode,
    }))
}

// ── Deduplication helpers ───────────────────────────────────────────────────

/// Score an address for dedup preference.  Lower is better.
///
/// Prefers traditional LAN IPs (192.168.x, 10.x) over Docker bridge addresses
/// and localhost.
fn score_address(addr: &str) -> u8 {
    if addr.starts_with("192.168.") || addr.starts_with("10.") {
        return 1;
    }
    if addr.starts_with("172.") {
        if !addr.starts_with("172.17.")
            && !addr.starts_with("172.18.")
            && !addr.starts_with("172.19.")
        {
            return 2;
        }
        return 3;
    }
    if addr.starts_with("host.docker.internal") {
        return 4;
    }
    if addr.starts_with("127.") || addr.starts_with("localhost") {
        return 5;
    }
    2
}

/// Deduplicate discovered servers, optionally filtering out backup-mode entries.
///
/// Servers are grouped by `(name, version)`.  When duplicates exist the
/// address with the best [`score_address`] wins.  When `keep_backups` is
/// false, any server that explicitly reported `mode = "backup"` is removed —
/// the discover endpoint is called by backup servers looking for a *primary*
/// to pair with.  When `keep_backups` is true (restore-from-backup flow),
/// backup servers are preserved in the results.
pub(crate) fn deduplicate_servers(
    discovered: Vec<serde_json::Value>,
    keep_backups: bool,
) -> Vec<serde_json::Value> {
    let mut dedup_map: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();

    for srv in discovered {
        let name = srv
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let version = srv
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let addr = srv
            .get("address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let key = format!("{}::{}", name, version);

        if let Some(existing) = dedup_map.get(&key) {
            let existing_addr = existing
                .get("address")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if score_address(&addr) < score_address(existing_addr) {
                dedup_map.insert(key, srv);
            }
        } else {
            dedup_map.insert(key, srv);
        }
    }

    if keep_backups {
        dedup_map.into_values().collect()
    } else {
        // Only return primary-mode servers; filter out anything that reported
        // mode="backup" explicitly. Servers with no mode field (backward compat)
        // are kept.
        dedup_map
            .into_values()
            .filter(|s| s.get("mode").and_then(|m| m.as_str()) != Some("backup"))
            .collect()
    }
}

// ── Probe helpers ───────────────────────────────────────────────────────────

/// Probe a single host:port for a Simple Photos server.
///
/// If `is_discovery` is true, probes `GET http://host:port/` (discovery protocol).
/// Otherwise probes `/api/discover/info` then `/health` as fallbacks.
/// `health_default_name` is used as the fallback server name for `/health` responses.
async fn probe_single(
    client: reqwest::Client,
    host: String,
    port: u16,
    is_discovery: bool,
    health_default_name: &str,
) -> Option<serde_json::Value> {
    if is_discovery {
        let url = format!("http://{}:{}/", host, port);
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    return parse_discovery_response(&body, &host, port);
                }
            }
        }
    } else {
        let info_url = format!("http://{}:{}/api/discover/info", host, port);
        let health_url = format!("http://{}:{}/health", host, port);

        if let Ok(resp) = client.get(&info_url).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(result) = parse_server_response(&body, &host, port, "Unknown") {
                        return Some(result);
                    }
                }
            }
        }
        if let Ok(resp) = client.get(&health_url).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(result) =
                        parse_server_response(&body, &host, port, health_default_name)
                    {
                        return Some(result);
                    }
                }
            }
        }
    }
    None
}

/// Collect probe results into `discovered` and `existing_addrs`, skipping duplicates.
fn collect_results(
    results: impl IntoIterator<Item = Option<serde_json::Value>>,
    discovered: &mut Vec<serde_json::Value>,
    existing_addrs: &mut std::collections::HashSet<String>,
) {
    for result in results.into_iter().flatten() {
        let addr = result
            .get("address")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        if !existing_addrs.contains(&addr) {
            existing_addrs.insert(addr);
            discovered.push(result);
        }
    }
}

// ── Phase 1: UDP broadcast ──────────────────────────────────────────────────

/// Discover servers via UDP broadcast (~1 second).
///
/// Catches backup-mode servers that are beaconing.  Results are tagged with
/// `mode: "backup"` so they are filtered out during final deduplication.
pub(crate) async fn phase1_udp_broadcast(
    discovered: &mut Vec<serde_json::Value>,
    existing_addrs: &mut std::collections::HashSet<String>,
) {
    let broadcast_results = tokio::task::spawn_blocking(|| {
        crate::backup::broadcast::discover_via_broadcast(std::time::Duration::from_secs(1))
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Broadcast discovery task panicked: {}", e);
        Vec::new()
    });

    for b in broadcast_results {
        if !existing_addrs.contains(&b.address) {
            existing_addrs.insert(b.address.clone());
            discovered.push(serde_json::json!({
                "address": b.address,
                "name": b.name,
                "version": b.version,
                "mode": "backup",
            }));
        }
    }
}

// ── Phase 2: Localhost / Docker probing ─────────────────────────────────────

/// Build the list of `(host, port, is_discovery)` probes for local addresses.
///
/// Probes the discovery port on well-known local hosts first, then falls back
/// to common API ports.  When the discovery port is active, fallback-port
/// probes on non-loopback hosts are skipped (they'll be found in Phase 3).
fn build_local_probes(
    discovery_port: u16,
    our_port: u16,
    existing_addrs: &std::collections::HashSet<String>,
) -> Vec<(String, u16, bool)> {
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

    let mut probes: Vec<(String, u16, bool)> = Vec::new();

    if discovery_port != 0 {
        for host in &probe_hosts {
            probes.push((host.clone(), discovery_port, true));
        }
        // Docker containers often map the internal discovery port (3301) to a
        // different host port (e.g. 3302, 3303).  Probe a small range around
        // the well-known port to catch those mappings.
        for offset in 1..=5u16 {
            let mapped_port = discovery_port + offset;
            if mapped_port != our_port {
                for host in &probe_hosts {
                    probes.push((host.clone(), mapped_port, true));
                }
            }
        }
    }

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
            probes.push((host.clone(), port, false));
        }
    }

    probes
}

/// Phase 2: Probe localhost and Docker-host addresses.
///
/// Uses a 1-second HTTP timeout — LAN servers respond in <10ms.
pub(crate) async fn phase2_local_probing(
    client: &reqwest::Client,
    discovery_port: u16,
    our_port: u16,
    discovered: &mut Vec<serde_json::Value>,
    existing_addrs: &mut std::collections::HashSet<String>,
) {
    let probes = build_local_probes(discovery_port, our_port, existing_addrs);

    let futures: Vec<_> = probes
        .into_iter()
        .map(|(host, port, is_discovery)| {
            probe_single(client.clone(), host, port, is_discovery, "Unknown")
        })
        .collect();

    let results = futures_util::future::join_all(futures).await;
    collect_results(results, discovered, existing_addrs);

    tracing::info!(
        "Setup discovery Phase 2 complete: {} servers found via local probes (discovery_port={})",
        discovered.len(),
        discovery_port
    );
}

// ── Phase 3: LAN subnet scan ───────────────────────────────────────────────

/// Gather /24 subnets to scan based on the server's base URL, local IP, and Docker host.
async fn gather_subnets(base_url: &str) -> Vec<String> {
    let mut subnets: Vec<String> = Vec::new();
    if let Ok(url) = reqwest::Url::parse(base_url) {
        if let Some(host) = url.host_str() {
            let parts: Vec<&str> = host.split('.').collect();
            if parts.len() == 4 {
                subnets.push(format!("{}.{}.{}", parts[0], parts[1], parts[2]));
            }
        }
    }
    if let Some(local_ip) = crate::backup::broadcast::get_local_ip() {
        tracing::debug!("Setup discovery: local IP = {}", local_ip);
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
    subnets
}

/// Build the list of `(ip, port, is_discovery)` probes for LAN subnet scanning.
fn build_subnet_probes(
    subnets: &[String],
    discovery_port: u16,
    our_port: u16,
    existing_addrs: &std::collections::HashSet<String>,
) -> Vec<(String, u16, bool)> {
    let mut probes: Vec<(String, u16, bool)> = Vec::new();
    for subnet in subnets {
        for host_part in 1..=254u8 {
            let ip = format!("{}.{}", subnet, host_part);
            if discovery_port != 0 {
                let addr = format!("{}:{}", ip, discovery_port);
                if !existing_addrs.contains(&addr) {
                    probes.push((ip.clone(), discovery_port, true));
                }
            }

            let mut sub_ports = vec![our_port];
            if our_port != 8080 {
                sub_ports.push(8080);
            }
            if discovery_port == 0 {
                sub_ports.extend(vec![8081, 8082, 8083, 3000]);
            }
            for port in sub_ports {
                let addr = format!("{}:{}", ip, port);
                if !existing_addrs.contains(&addr) {
                    probes.push((ip.clone(), port, false));
                }
            }
        }
    }
    probes
}

/// Phase 3: Scan LAN subnets for Simple Photos servers.
///
/// Sweeps `x.x.x.1-254` on the discovery port plus common fallback ports.
/// Uses a semaphore (200 concurrent) and a 12-second deadline with streaming
/// collection so partial results survive timeouts.
pub(crate) async fn phase3_subnet_scan(
    client: &reqwest::Client,
    base_url: &str,
    discovery_port: u16,
    our_port: u16,
    discovered: &mut Vec<serde_json::Value>,
    existing_addrs: &mut std::collections::HashSet<String>,
) {
    let subnets = gather_subnets(base_url).await;
    let probes = build_subnet_probes(&subnets, discovery_port, our_port, existing_addrs);

    tracing::info!(
        "Setup discovery Phase 3: scanning {} subnets ({:?}) on port {}, {} total probes",
        subnets.len(),
        subnets,
        if discovery_port != 0 { discovery_port } else { our_port },
        probes.len()
    );

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(200));

    let futures: Vec<_> = probes
        .into_iter()
        .map(|(ip, port, is_discovery)| {
            let c = client.clone();
            let sem = sem.clone();
            async move {
                let _permit = sem.acquire().await;
                probe_single(c, ip, port, is_discovery, "Simple Photos").await
            }
        })
        .collect();

    // Use FuturesUnordered + streaming so we collect results as they
    // complete.  Previously `join_all` discarded ALL results when the
    // timeout fired — even those already finished.
    let mut stream: futures_util::stream::FuturesUnordered<_> = futures.into_iter().collect();

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        match tokio::time::timeout_at(deadline, futures_util::StreamExt::next(&mut stream)).await {
            Ok(Some(Some(result))) => {
                let addr = result
                    .get("address")
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .to_string();
                if !existing_addrs.contains(&addr) {
                    existing_addrs.insert(addr);
                    discovered.push(result);
                }
            }
            Ok(Some(None)) => { /* probe returned None */ }
            Ok(None) => break,
            Err(_) => {
                tracing::warn!(
                    "Setup discovery: LAN scan timed out after 12s with {} servers found so far",
                    discovered.len()
                );
                break;
            }
        }
    }
}
