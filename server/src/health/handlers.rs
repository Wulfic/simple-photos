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

/// GET /api/status/activity — report whether server-side background work
/// (AI inference, geo backfill) is currently running **or has pending
/// work for the authenticated user**.
///
/// The flag is the OR of two signals:
/// 1. The transient atomic flag set by the AI/geo processors while a
///    batch is in flight.  This is precise but flickers — a sub-second
///    batch will rarely be observable to a 3-second poll.
/// 2. A backlog query: any photo belonging to this user that still
///    needs detection or reverse-geocoding.  This makes the spinner
///    visible for the entire duration of background work, not just the
///    moment a batch is actively running.
///
/// Requires authentication so the activity state isn't leaked to anonymous
/// callers.  Used by the web client to spin the profile-avatar indicator
/// so users know when the server is doing something on their behalf.
pub async fn activity_status(
    State(state): State<AppState>,
    auth: crate::auth::middleware::AuthUser,
) -> Json<Value> {
    use std::sync::atomic::Ordering;

    let ai_running = state.ai_active.load(Ordering::Relaxed);
    let geo_running = state.geo_active.load(Ordering::Relaxed);

    // Backlog: photos awaiting AI detection for this user.  We only count
    // a small LIMIT to keep the query bounded — we don't need an exact
    // total, just "is there at least one?".  Errors are swallowed so a
    // transient DB hiccup doesn't 500 a status poll.
    //
    // IMPORTANT: mirror the AI processor's user-enabled filter exactly.
    // Without it, photos belonging to users who have AI disabled (or who
    // are on a server where AI is globally off) are never processed, so
    // the backlog query would return true forever and the spinner would
    // never stop.
    let ai_config_default = if state.config.ai.enabled { 1i64 } else { 0i64 };
    let ai_pending: bool = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS( \
             SELECT 1 FROM photos p \
             WHERE p.user_id = ?1 \
             AND ( (p.file_path IS NOT NULL AND p.file_path != '') OR p.encrypted_blob_id IS NOT NULL ) \
             AND NOT EXISTS ( \
                 SELECT 1 FROM ai_processed_photos ap \
                 WHERE ap.photo_id = p.id AND ap.user_id = p.user_id \
             ) \
             AND ( \
                 EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'ai_enabled' AND us.value = 'true') \
                 OR ( \
                     ?2 = 1 AND NOT EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'ai_enabled') \
                 ) \
             ) \
             LIMIT 1 \
         )",
    )
    .bind(&auth.user_id)
    .bind(ai_config_default)
    .fetch_one(&state.read_pool)
    .await
    .map(|n| n != 0)
    .unwrap_or(false);

    // Backlog: photos with GPS but no resolved geo_city for this user.
    // Again, mirror the geo processor's user-enabled filter so we don't
    // report pending work for users who have geo disabled — those photos
    // will never get geo_city populated and would spin forever otherwise.
    let geo_config_default = if state.config.geo.enabled { 1i64 } else { 0i64 };
    let geo_pending: bool = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS( \
             SELECT 1 FROM photos p \
             WHERE p.user_id = ?1 \
             AND p.latitude IS NOT NULL AND p.longitude IS NOT NULL \
             AND p.geo_city IS NULL \
             AND ( \
                 EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'geo_enabled' AND us.value = 'true') \
                 OR ( \
                     ?2 = 1 AND NOT EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'geo_enabled') \
                 ) \
             ) \
             LIMIT 1 \
         )",
    )
    .bind(&auth.user_id)
    .bind(geo_config_default)
    .fetch_one(&state.read_pool)
    .await
    .map(|n| n != 0)
    .unwrap_or(false);

    let ai = ai_running || ai_pending;
    let geo = geo_running || geo_pending;
    Json(json!({
        "ai": ai,
        "geo": geo,
        "active": ai || geo,
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
