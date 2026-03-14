//! UDP LAN discovery beacon for backup-mode servers.
//!
//! When backup mode is enabled, this module broadcasts a periodic UDP
//! beacon on port 41820 so primary servers can auto-discover backup
//! servers on the local network.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

/// UDP port used for Simple Photos server discovery broadcast.
pub const DISCOVERY_PORT: u16 = 41820;

/// Magic prefix so we only parse our own broadcast packets.
const MAGIC: &[u8; 4] = b"SPBK";

/// A server discovered via UDP broadcast.
#[derive(Debug, Clone)]
pub struct BroadcastInfo {
    pub address: String,
    pub name: String,
    pub version: String,
    pub api_key_required: bool,
}

// ── Broadcast Listener (Backup-Mode Servers) ─────────────────────────────────

/// Background task: when `backup_mode` is enabled, this server periodically
/// broadcasts a UDP beacon on the LAN so primary servers can auto-discover it.
///
/// Broadcast payload (UTF-8):  SPBK|<port>|<name>|<version>|<api_key_required>
pub async fn background_broadcast_task(
    pool: sqlx::SqlitePool,
    server_port: u16,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        interval.tick().await;

        // Check if this server is in backup mode
        let mode: Option<String> = sqlx::query_scalar(
            "SELECT value FROM server_settings WHERE key = 'backup_mode'",
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();

        if mode.as_deref() != Some("backup") {
            // Not in backup mode — sleep longer and recheck
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        }

        // Check if an API key is configured
        let api_key_set: bool = sqlx::query_scalar::<_, Option<String>>(
            "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .map(|v| !v.is_empty())
        .unwrap_or(false);

        // Build the broadcast payload
        let payload = format!(
            "SPBK|{}|Simple Photos Backup|{}|{}",
            server_port,
            crate::VERSION,
            if api_key_set { "1" } else { "0" }
        );

        // Send UDP broadcast to the discovery port
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
            let _ = socket.set_broadcast(true);
            let dest = SocketAddrV4::new(Ipv4Addr::BROADCAST, DISCOVERY_PORT);
            let _ = socket.send_to(payload.as_bytes(), dest);
        }
    }
}

// ── Discovery via UDP Broadcast ──────────────────────────────────────────────

/// Listen for broadcast beacons from backup-mode servers.
/// Returns all servers that responded within the timeout window.
pub fn discover_via_broadcast(timeout: Duration) -> Vec<BroadcastInfo> {
    let mut results = Vec::new();

    // Bind to the discovery port to receive broadcasts
    let socket = match UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)) {
        Ok(s) => s,
        Err(_) => {
            // Port might be in use (e.g., our own broadcast listener). Try ephemeral.
            match UdpSocket::bind("0.0.0.0:0") {
                Ok(s) => s,
                Err(_) => return results,
            }
        }
    };

    let _ = socket.set_read_timeout(Some(timeout));
    let _ = socket.set_broadcast(true);

    // Also send a discovery probe so backup servers can respond immediately
    let probe = b"SPBK|DISCOVER";
    let dest = SocketAddrV4::new(Ipv4Addr::BROADCAST, DISCOVERY_PORT);
    let _ = socket.send_to(probe, dest);

    let deadline = std::time::Instant::now() + timeout;
    let mut buf = [0u8; 512];

    while std::time::Instant::now() < deadline {
        match socket.recv_from(&mut buf) {
            Ok((n, addr)) => {
                if n < 4 || &buf[..4] != MAGIC {
                    continue;
                }
                let payload = match std::str::from_utf8(&buf[..n]) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                // Format: SPBK|<port>|<name>|<version>|<api_key_required>
                let parts: Vec<&str> = payload.split('|').collect();
                if parts.len() < 5 || parts[1] == "DISCOVER" {
                    continue;
                }

                let port: u16 = match parts[1].parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                let ip = addr.ip();
                let address = format!("{}:{}", ip, port);

                // Deduplicate
                if results.iter().any(|r: &BroadcastInfo| r.address == address) {
                    continue;
                }

                results.push(BroadcastInfo {
                    address,
                    name: parts[2].to_string(),
                    version: parts[3].to_string(),
                    api_key_required: parts[4] == "1",
                });
            }
            Err(_) => break, // Timeout or error
        }
    }

    results
}

// ── Get Local IP Address ─────────────────────────────────────────────────────

/// Returns the local IP address of this machine by opening a UDP socket
/// and reading the local address. This doesn't send any data.
pub fn get_local_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    // Connect to an external address — doesn't actually send anything
    socket.connect("8.8.8.8:80").ok()?;
    let local_addr = socket.local_addr().ok()?;
    Some(local_addr.ip().to_string())
}
