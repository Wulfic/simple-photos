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
/// work for the authenticated user**, plus per-task progress counts so
/// the web client can render banners with totals/done/percent — the same
/// pattern used by the encryption and conversion banners.
///
/// Each task block reports:
///   - `running`  — transient flag set by the processor mid-batch
///   - `pending`  — count of photos still requiring this work
///   - `total`    — count of photos in scope (eligible for this work)
///   - `done`     — `total - pending`
///   - `active`   — `running || (pending > 0)`
///
/// Requires authentication so the activity state isn't leaked to anonymous
/// callers.  Used by the web client to spin the profile-avatar indicator
/// and drive AI / geo progress banners.
pub async fn activity_status(
    State(state): State<AppState>,
    auth: crate::auth::middleware::AuthUser,
) -> Json<Value> {
    use std::sync::atomic::Ordering;

    let ai_running = state.ai_active.load(Ordering::Relaxed);
    let geo_running = state.geo_active.load(Ordering::Relaxed);

    let ai_config_default = if state.config.ai.enabled { 1i64 } else { 0i64 };

    // Total photos in AI scope for this user (mirrors processor's
    // user-enabled filter).  Errors are swallowed so a transient DB hiccup
    // doesn't 500 a status poll.
    let ai_total: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM photos p \
             WHERE p.user_id = ?1 \
             AND ( (p.file_path IS NOT NULL AND p.file_path != '') OR p.encrypted_blob_id IS NOT NULL ) \
             AND ( \
                 EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'ai_enabled' AND us.value = 'true') \
                 OR ( \
                     ?2 = 1 AND NOT EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'ai_enabled') \
                 ) \
             )",
    )
    .bind(&auth.user_id)
    .bind(ai_config_default)
    .fetch_one(&state.read_pool)
    .await
    .unwrap_or(0);

    let ai_pending_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM photos p \
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
             )",
    )
    .bind(&auth.user_id)
    .bind(ai_config_default)
    .fetch_one(&state.read_pool)
    .await
    .unwrap_or(0);

    // Geo scope: photos with GPS coordinates and the user's geo enabled.
    let geo_config_default = if state.config.geo.enabled { 1i64 } else { 0i64 };
    let geo_total: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM photos p \
             WHERE p.user_id = ?1 \
             AND p.latitude IS NOT NULL AND p.longitude IS NOT NULL \
             AND ( \
                 EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'geo_enabled' AND us.value = 'true') \
                 OR ( \
                     ?2 = 1 AND NOT EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'geo_enabled') \
                 ) \
             )",
    )
    .bind(&auth.user_id)
    .bind(geo_config_default)
    .fetch_one(&state.read_pool)
    .await
    .unwrap_or(0);

    let geo_pending_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM photos p \
             WHERE p.user_id = ?1 \
             AND p.latitude IS NOT NULL AND p.longitude IS NOT NULL \
             AND p.geo_city IS NULL \
             AND ( \
                 EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'geo_enabled' AND us.value = 'true') \
                 OR ( \
                     ?2 = 1 AND NOT EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = p.user_id AND us.key = 'geo_enabled') \
                 ) \
             )",
    )
    .bind(&auth.user_id)
    .bind(geo_config_default)
    .fetch_one(&state.read_pool)
    .await
    .unwrap_or(0);

    let ai_done = (ai_total - ai_pending_count).max(0);
    let geo_done = (geo_total - geo_pending_count).max(0);
    let ai_active = ai_running || ai_pending_count > 0;
    let geo_active = geo_running || geo_pending_count > 0;

    Json(json!({
        // Back-compat top-level booleans (used by older clients).
        "ai": ai_active,
        "geo": geo_active,
        "active": ai_active || geo_active,
        // Per-task progress, banner-friendly.
        "ai_progress": {
            "running": ai_running,
            "active": ai_active,
            "total": ai_total,
            "done": ai_done,
            "pending": ai_pending_count,
        },
        "geo_progress": {
            "running": geo_running,
            "active": geo_active,
            "total": geo_total,
            "done": geo_done,
            "pending": geo_pending_count,
        },
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
            Some(format!("{host}:{port}"))
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

/// Returns `true` if the IP belongs to the Docker default-bridge range
/// (`172.16.0.0/12`), used by co-located backup containers reaching this
/// server through Docker's NAT.
///
/// **Security:** this deliberately does **not** include `10.0.0.0/8`. That
/// range covers the majority of ordinary home/corporate LANs and VPNs, so
/// trusting it would expose the backup `api_key` (returned by
/// [`discover_info`] in backup mode) to any host on such a network. Docker
/// Swarm / Kubernetes overlay deployments that use `10.x` should pair via the
/// authenticated `/api/setup/pair` flow instead of relying on this
/// unauthenticated discovery shortcut.
fn is_docker_internal(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 172.16.0.0/12 — Docker default bridge and custom bridge networks.
            octets[0] == 172 && (16..=31).contains(&octets[1])
        }
        std::net::IpAddr::V6(_) => false,
    }
}
