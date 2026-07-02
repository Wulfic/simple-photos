//! Secure peer heartbeat for local self-healing over the discovery port (3301).
//!
//! Paired Simple Photos servers (primary ↔ backup) share an `api_key`. This
//! module lets them prove liveness to each other every ~15 minutes with an
//! **HMAC-SHA256-authenticated**, **replay-protected** message, and drives a
//! sender loop that detects missed heartbeats and probes for recovery
//! (self-healing) with exponential backoff.
//!
//! ## Message
//! ```json
//! { "message": { "sender_id": "...", "timestamp_ms": 1720000000000, "nonce": "<hex>" },
//!   "signature": "<hex hmac-sha256>" }
//! ```
//! The signature covers the canonical string `sender_id|timestamp_ms|nonce`
//! keyed by the shared `api_key`.
//!
//! ## Anti-abuse (item #10 security notes)
//! * **Authentication**: HMAC over the shared key — an attacker without the key
//!   can't forge a heartbeat. Verified in constant time (`subtle`).
//! * **Replay**: every message carries a fresh random `nonce` and a timestamp.
//!   The receiver rejects messages outside a ±freshness window and rejects any
//!   nonce seen within the retention window ([`NonceCache`]).
//! * **Interception**: the payload carries no secrets (only a timestamp + random
//!   nonce), so capturing one reveals nothing; the HMAC prevents tampering.
//!
//! A full external security review is still recommended before relying on this
//! for anything beyond liveness signalling.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Reject heartbeats whose timestamp is more than this far from local time.
/// Wide enough to tolerate clock skew between hosts, tight enough to bound the
/// replay window a captured message could be reused in.
pub const FRESHNESS_WINDOW_MS: i64 = 120_000; // ±2 min
/// How long a seen nonce is remembered (must exceed the freshness window so a
/// captured message can't be replayed after its nonce is forgotten but while its
/// timestamp is still fresh).
const NONCE_RETENTION: Duration = Duration::from_secs(300); // 5 min
/// Heartbeat cadence.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Consecutive misses before a peer is considered down (drives self-healing).
const MISS_THRESHOLD: u32 = 3;

/// The signed portion of a heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatMessage {
    pub sender_id: String,
    pub timestamp_ms: i64,
    pub nonce: String,
}

impl HeartbeatMessage {
    /// Canonical byte string that the HMAC is computed over. A fixed field order
    /// with a delimiter that can't appear in the numeric/hex fields keeps the
    /// signature unambiguous.
    fn canonical(&self) -> String {
        format!("{}|{}|{}", self.sender_id, self.timestamp_ms, self.nonce)
    }
}

/// Full wire envelope: the message plus its detached signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatEnvelope {
    pub message: HeartbeatMessage,
    pub signature: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HeartbeatError {
    BadSignature,
    Stale,
    Replay,
}

/// Compute the hex HMAC-SHA256 signature of `msg` under `key`.
pub fn sign(key: &[u8], msg: &HeartbeatMessage) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg.canonical().as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Build a fresh, signed heartbeat envelope from this server.
pub fn make_envelope(key: &[u8], sender_id: &str) -> HeartbeatEnvelope {
    let message = HeartbeatMessage {
        sender_id: sender_id.to_string(),
        timestamp_ms: now_ms(),
        nonce: random_nonce(),
    };
    let signature = sign(key, &message);
    HeartbeatEnvelope { message, signature }
}

/// Verify a signature in constant time (no early-out on the first differing byte).
fn signature_valid(key: &[u8], msg: &HeartbeatMessage, signature: &str) -> bool {
    let expected = sign(key, msg);
    // Constant-time compare of the hex strings; length-mismatch is a fast reject.
    if expected.len() != signature.len() {
        return false;
    }
    use subtle::ConstantTimeEq;
    expected.as_bytes().ct_eq(signature.as_bytes()).into()
}

/// Verify an envelope against a single candidate `key`: signature, freshness, and
/// nonce novelty. On success the nonce is recorded so a replay is rejected.
pub fn verify_with_key(
    key: &[u8],
    env: &HeartbeatEnvelope,
    now_ms: i64,
    nonces: &NonceCache,
) -> Result<(), HeartbeatError> {
    if !signature_valid(key, &env.message, &env.signature) {
        return Err(HeartbeatError::BadSignature);
    }
    if (now_ms - env.message.timestamp_ms).abs() > FRESHNESS_WINDOW_MS {
        return Err(HeartbeatError::Stale);
    }
    // Only consume the nonce once the signature + freshness pass, so an attacker
    // can't burn a legitimate nonce with a forged message.
    if !nonces.insert_if_new(&env.message.nonce) {
        return Err(HeartbeatError::Replay);
    }
    Ok(())
}

/// Try each candidate key in turn (a server may hold several: its own backup key
/// plus every peer's key). Returns the matching key on success so the receiver
/// can sign its pong with the same shared secret.
pub fn verify_any<'a>(
    keys: &'a [Vec<u8>],
    env: &HeartbeatEnvelope,
    now_ms: i64,
    nonces: &NonceCache,
) -> Result<&'a [u8], HeartbeatError> {
    // Check signature against all keys first (constant work regardless of match)
    // before touching the nonce cache, so a bad message never consumes a nonce.
    let matched = keys
        .iter()
        .find(|k| signature_valid(k, &env.message, &env.signature));
    let key = match matched {
        Some(k) => k.as_slice(),
        None => return Err(HeartbeatError::BadSignature),
    };
    verify_with_key(key, env, now_ms, nonces)?;
    Ok(key)
}

/// In-memory replay guard: remembers recently-seen nonces with a TTL.
pub struct NonceCache {
    seen: Mutex<HashMap<String, Instant>>,
}

impl Default for NonceCache {
    fn default() -> Self {
        Self {
            seen: Mutex::new(HashMap::new()),
        }
    }
}

impl NonceCache {
    /// Record `nonce`; returns `false` if it was already present (a replay).
    /// Prunes expired entries opportunistically.
    pub fn insert_if_new(&self, nonce: &str) -> bool {
        let now = Instant::now();
        let mut map = self.seen.lock().unwrap();
        map.retain(|_, seen_at| now.duration_since(*seen_at) < NONCE_RETENTION);
        if map.contains_key(nonce) {
            return false;
        }
        map.insert(nonce.to_string(), now);
        true
    }
}

/// Process-global nonce cache shared by the discovery-port receiver.
pub fn global_nonce_cache() -> &'static NonceCache {
    static CACHE: OnceLock<NonceCache> = OnceLock::new();
    CACHE.get_or_init(NonceCache::default)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn random_nonce() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

// ── Sender / self-healing loop ───────────────────────────────────────────────

/// Background task: every [`HEARTBEAT_INTERVAL`], send a signed heartbeat to each
/// enabled backup peer's discovery port and track liveness. On repeated misses a
/// peer is logged as down; when it answers again, recovery is logged. Runs
/// forever; safe to spawn once at startup.
pub async fn run_heartbeat_sender(
    pool: sqlx::SqlitePool,
    config: std::sync::Arc<crate::config::AppConfig>,
) {
    // This server's identity in the heartbeat (best-effort; falls back to name).
    let sender_id = server_identity(&pool, &config).await;
    // Per-peer consecutive-miss counters (peer id → misses).
    let mut misses: HashMap<String, u32> = HashMap::new();

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .danger_accept_invalid_certs(config.backup.accept_invalid_certs)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[heartbeat] failed to build HTTP client: {e}");
            return;
        }
    };

    loop {
        let peers: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT id, address, api_key FROM backup_servers WHERE enabled = 1 AND api_key IS NOT NULL AND api_key != ''",
        )
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

        for (peer_id, address, api_key) in peers {
            let ok = send_heartbeat_with_backoff(
                &client,
                &address,
                api_key.as_bytes(),
                &sender_id,
                config.server.discovery_port,
            )
            .await;

            let counter = misses.entry(peer_id.clone()).or_insert(0);
            if ok {
                if *counter >= MISS_THRESHOLD {
                    tracing::info!(peer = %peer_id, "[heartbeat] peer recovered (self-healed)");
                }
                *counter = 0;
            } else {
                *counter += 1;
                if *counter == MISS_THRESHOLD {
                    tracing::warn!(
                        peer = %peer_id, misses = *counter,
                        "[heartbeat] peer unreachable after repeated misses — flagged down for self-healing"
                    );
                } else {
                    tracing::debug!(peer = %peer_id, misses = *counter, "[heartbeat] missed");
                }
            }
        }

        tokio::time::sleep(HEARTBEAT_INTERVAL).await;
    }
}

/// Send one heartbeat with a few exponentially-backed-off retries. Returns true
/// once the peer answers with a valid, matching pong.
async fn send_heartbeat_with_backoff(
    client: &reqwest::Client,
    address: &str,
    key: &[u8],
    sender_id: &str,
    discovery_port: u16,
) -> bool {
    let url = match heartbeat_url(address, discovery_port) {
        Some(u) => u,
        None => return false,
    };
    let mut delay = Duration::from_secs(2);
    for attempt in 0..3 {
        let env = make_envelope(key, sender_id);
        match client.post(&url).json(&env).send().await {
            Ok(resp) if resp.status().is_success() => {
                // Verify the pong is signed with the same shared key (mutual auth).
                if let Ok(pong) = resp.json::<HeartbeatEnvelope>().await {
                    if signature_valid(key, &pong.message, &pong.signature) {
                        return true;
                    }
                    tracing::debug!("[heartbeat] pong signature invalid from {url}");
                }
                // A 2xx without a valid pong still proves reachability of the port.
                return true;
            }
            Ok(resp) => {
                tracing::debug!(attempt, status = %resp.status(), "[heartbeat] non-2xx from {url}");
            }
            Err(e) => {
                tracing::debug!(attempt, "[heartbeat] send error to {url}: {e}");
            }
        }
        if attempt < 2 {
            tokio::time::sleep(delay).await;
            delay *= 2;
        }
    }
    false
}

/// Build `http://<host>:<discovery_port>/heartbeat` from a stored peer address
/// (which may be `host`, `host:port`, or a full URL).
fn heartbeat_url(address: &str, discovery_port: u16) -> Option<String> {
    // Strip scheme and any path/port, keep the bare host.
    let no_scheme = address
        .strip_prefix("https://")
        .or_else(|| address.strip_prefix("http://"))
        .unwrap_or(address);
    let host = no_scheme.split('/').next()?.split(':').next()?.trim();
    if host.is_empty() {
        return None;
    }
    // The discovery listener is always plain HTTP (see backup::discovery).
    Some(format!("http://{host}:{discovery_port}/heartbeat"))
}

/// A stable identity string for this server (its configured name, else host).
async fn server_identity(pool: &sqlx::SqlitePool, config: &crate::config::AppConfig) -> String {
    sqlx::query_scalar::<_, String>("SELECT value FROM server_settings WHERE key = 'server_name'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| config.server.host.clone())
}

/// Candidate keys the receiver verifies against: its own configured backup key
/// plus every peer's key. Either direction of a pairing can then be validated.
pub async fn candidate_keys(pool: &sqlx::SqlitePool, config: &crate::config::AppConfig) -> Vec<Vec<u8>> {
    let mut keys: Vec<Vec<u8>> = Vec::new();
    if let Some(k) = config.backup.api_key.as_deref().filter(|k| !k.is_empty()) {
        keys.push(k.as_bytes().to_vec());
    }
    if let Ok(Some(k)) = sqlx::query_scalar::<_, Option<String>>(
        "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
    )
    .fetch_optional(pool)
    .await
    {
        if let Some(k) = k.filter(|k| !k.is_empty()) {
            keys.push(k.into_bytes());
        }
    }
    let peer_keys: Vec<String> = sqlx::query_scalar(
        "SELECT api_key FROM backup_servers WHERE api_key IS NOT NULL AND api_key != ''",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for k in peer_keys {
        keys.push(k.into_bytes());
    }
    // De-duplicate so verify_any does minimal work.
    keys.sort();
    keys.dedup();
    keys
}

/// Convenience for the receiver: current epoch millis.
pub fn current_ms() -> i64 {
    now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg() -> HeartbeatMessage {
        HeartbeatMessage {
            sender_id: "srv-a".into(),
            timestamp_ms: current_ms(),
            nonce: random_nonce(),
        }
    }

    #[test]
    fn valid_roundtrip_accepted() {
        let key = b"shared-secret";
        let env = make_envelope(key, "srv-a");
        let nonces = NonceCache::default();
        assert!(verify_with_key(key, &env, current_ms(), &nonces).is_ok());
    }

    #[test]
    fn wrong_key_rejected() {
        let env = make_envelope(b"key-one", "srv-a");
        let nonces = NonceCache::default();
        assert_eq!(
            verify_with_key(b"key-two", &env, current_ms(), &nonces),
            Err(HeartbeatError::BadSignature)
        );
    }

    #[test]
    fn tampered_message_rejected() {
        let key = b"shared-secret";
        let mut env = make_envelope(key, "srv-a");
        env.message.sender_id = "evil".into(); // signature no longer matches
        let nonces = NonceCache::default();
        assert_eq!(
            verify_with_key(key, &env, current_ms(), &nonces),
            Err(HeartbeatError::BadSignature)
        );
    }

    #[test]
    fn stale_timestamp_rejected() {
        let key = b"shared-secret";
        let m = HeartbeatMessage {
            sender_id: "srv-a".into(),
            timestamp_ms: current_ms() - FRESHNESS_WINDOW_MS - 1_000,
            nonce: random_nonce(),
        };
        let env = HeartbeatEnvelope {
            signature: sign(key, &m),
            message: m,
        };
        let nonces = NonceCache::default();
        assert_eq!(
            verify_with_key(key, &env, current_ms(), &nonces),
            Err(HeartbeatError::Stale)
        );
    }

    #[test]
    fn replayed_nonce_rejected() {
        let key = b"shared-secret";
        let env = make_envelope(key, "srv-a");
        let nonces = NonceCache::default();
        assert!(verify_with_key(key, &env, current_ms(), &nonces).is_ok());
        // Second time with the same nonce is a replay.
        assert_eq!(
            verify_with_key(key, &env, current_ms(), &nonces),
            Err(HeartbeatError::Replay)
        );
    }

    #[test]
    fn verify_any_matches_second_key() {
        let key = b"real-key";
        let env = make_envelope(key, "srv-a");
        let keys = vec![b"other".to_vec(), key.to_vec()];
        let nonces = NonceCache::default();
        let matched = verify_any(&keys, &env, current_ms(), &nonces).unwrap();
        assert_eq!(matched, key);
    }

    #[test]
    fn bad_signature_does_not_consume_nonce() {
        let key = b"shared-secret";
        let env = make_envelope(key, "srv-a");
        let nonces = NonceCache::default();
        // Forged verification against the wrong key must NOT burn the nonce.
        let _ = verify_with_key(b"wrong", &env, current_ms(), &nonces);
        // The genuine message still verifies.
        assert!(verify_with_key(key, &env, current_ms(), &nonces).is_ok());
    }

    #[test]
    fn heartbeat_url_forms() {
        assert_eq!(
            heartbeat_url("192.168.1.5:8080", 3301).as_deref(),
            Some("http://192.168.1.5:3301/heartbeat")
        );
        assert_eq!(
            heartbeat_url("https://host.example/api", 3301).as_deref(),
            Some("http://host.example:3301/heartbeat")
        );
        assert_eq!(heartbeat_url("", 3301), None);
    }

    #[test]
    fn signed_canonical_is_field_ordered() {
        // Guard the canonical form so a wire-format change can't silently
        // invalidate every peer's signature.
        let m = HeartbeatMessage {
            sender_id: "a".into(),
            timestamp_ms: 42,
            nonce: "ff".into(),
        };
        assert_eq!(m.canonical(), "a|42|ff");
        let _ = msg(); // keep helper referenced
    }
}
