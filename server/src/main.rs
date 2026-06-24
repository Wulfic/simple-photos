//! Entry point for the Simple Photos server.
//!
//! Startup sequence:
//! 1. Load and validate configuration (TOML + env var overrides)
//! 2. Initialize SQLite database pool and run migrations
//! 3. Launch background tasks:
//!    - Rate-limiter stale-entry cleanup (every 5 min)
//!    - Housekeeping: expired token purge, audit-log trim (90 days),
//!      client-diagnostic-log trim (14 days) — runs hourly
//!    - Trash auto-purge (expired items, hourly)
//!    - Backup server sync (hourly check per server frequency)
//!    - Storage auto-scan (configurable interval)
//!    - UDP broadcast for LAN backup-server discovery
//! 4. Build the Axum router with all API routes, middleware, and static
//!    file serving
//! 5. Bind to HTTP or HTTPS (if TLS configured) and start accepting
//!    connections

// Stabilization branch: silence stylistic clippy lints crate-wide so the
// CI gate stays green. Correctness/security lints (unwrap_used, panic,
// undocumented_unsafe_blocks, etc.) remain enforced.
#![allow(clippy::type_complexity)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::result_large_err)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::let_and_return)]
#![allow(clippy::doc_overindented_list_items)]

mod ai;
mod audit;
mod auth;
mod backup;
mod blobs;
mod client_logs;
mod config;
mod conversion;
mod crypto;
mod db;
mod diagnostics;
mod downloads;
mod editing;
mod error;
mod export;
mod gallery;
mod geo;
mod health;
mod http_utils;
mod import;
mod ingest;
mod media;
mod photos;
mod process;
mod ratelimit;
mod routes;
mod sanitize;
mod security;
mod setup;
mod sharing;
mod state;
mod tags;
mod tasks;
mod transcode;
mod trash;

/// Server version, read from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::ratelimit::RateLimiters;
use crate::state::AppState;

/// Tokio multi-threaded runtime with a floor of 4 worker threads.
///
/// On machines with fewer than 4 cores (e.g. Raspberry Pi, small VPS), the
/// default `num_cpus` worker count is too low — a single `spawn_blocking`
/// call or CPU-bound operation can starve the request pipeline. The floor
/// ensures adequate parallelism for concurrent uploads + gallery serving.
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    // Initialize structured logging. Uses RUST_LOG env var if set
    // (e.g. RUST_LOG=debug), otherwise defaults to "info" level.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // ── rustls process-level CryptoProvider ──────────────────────────────
    // Multiple rustls-using crates (axum-server-tls, instant-acme, reqwest)
    // pull in different crypto providers via their default feature sets.
    // When more than one is present the auto-selection panics, so we pin
    // the `ring` provider explicitly here.  Idempotent: subsequent calls
    // (e.g. by tests) are no-ops.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = config::AppConfig::load()?;
    tracing::info!("Starting Simple Photos server v{VERSION}");
    tracing::info!("Listening on {}:{}", config.server.host, config.server.port);
    tracing::info!("Storage root: {:?}", config.storage.root);
    tracing::info!(
        "Max blob size: {} MiB",
        config.storage.max_blob_size_bytes / (1024 * 1024)
    );

    // Validate JWT secret strength at startup
    if config.auth.jwt_secret.len() < 32 {
        tracing::error!("JWT secret is too short (< 32 chars). Use: openssl rand -hex 32");
        anyhow::bail!("JWT secret must be at least 32 characters for security.");
    }
    if config.auth.jwt_secret == "CHANGE_ME_RANDOM_64_CHAR_HEX" {
        tracing::error!("JWT secret is the default placeholder — this is insecure!");
        anyhow::bail!("Please set a real JWT secret. Generate one with: openssl rand -hex 32");
    }

    // If `[storage.smb]` is configured, ensure the share is mounted before we
    // touch the storage tree. A failure here is logged but non-fatal — the
    // storage health monitor will mark the backend unavailable, the wizard
    // and admin UI surface a clear error, and once the operator fixes the
    // mount the server reconnects automatically.
    if let Some(smb_cfg) = &config.storage.smb {
        if let Err(e) = remount_smb_on_boot(smb_cfg, &config.auth.jwt_secret).await {
            tracing::error!("SMB share remount failed: {}. Server will continue with the configured local path; \
                              fix the share and restart, or reconfigure storage from the admin UI.", e);
        } else {
            tracing::info!(
                "SMB share `{}` mounted at {} (storage root: {:?})",
                smb_cfg.address,
                smb_cfg.mount_point,
                config.storage.root
            );
        }
    }

    // Ensure storage directory tree exists (network mounts should already be present)
    tokio::fs::create_dir_all(&config.storage.root).await?;
    // Create organized subdirectories under the storage root
    tokio::fs::create_dir_all(config.storage.root.join("blobs")).await?;
    tokio::fs::create_dir_all(config.storage.root.join("metadata")).await?;
    tokio::fs::create_dir_all(config.storage.root.join("logs/server")).await?;
    tokio::fs::create_dir_all(config.storage.root.join("logs/app")).await?;
    tracing::info!(
        "Storage subdirectories initialized: blobs/, metadata/, logs/server/, logs/app/"
    );

    let (pool, read_pool) = db::init_pools(&config.database, &config.auth.jwt_secret).await?;

    // Initialize in-memory per-IP rate limiters for auth endpoints,
    // and start a background task that evicts stale entries every 5 min.
    let rate_limiters = RateLimiters::new();
    rate_limiters.spawn_cleanup_task();

    let scan_lock: Arc<tokio::sync::Mutex<()>> = Arc::new(tokio::sync::Mutex::new(()));

    // ArcSwap lets background tasks and handlers read the *current* storage
    // root — respecting runtime changes via the setup wizard.
    let storage_root_swap = Arc::new(arc_swap::ArcSwap::from_pointee(config.storage.root.clone()));

    // Broadcast channel for real-time audit log events (SSE + backup forwarding).
    let (audit_tx, _) = tokio::sync::broadcast::channel(256);

    // Launch all background tasks (housekeeping, backup sync, auto-scan, etc.)
    let storage_available = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let ai_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let geo_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Optimistic default: assume the dataset is fine until the geo processor
    // actually tries (and possibly fails) to load it.  Avoids a spurious
    // "unavailable" flash on boot before the first poll cycle.
    let geo_dataset_available = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let geo_dataset_downloading = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Lets handlers (settings toggle, upload) and the auto-scan task wake the
    // geo processor on demand rather than waiting for its next 5-min poll tick.
    let geo_trigger = Arc::new(tokio::sync::Notify::new());
    tasks::spawn_all(
        &pool,
        &config,
        &storage_root_swap,
        &scan_lock,
        &audit_tx,
        &storage_available,
        &ai_active,
        &geo_active,
        &geo_dataset_available,
        &geo_dataset_downloading,
        &geo_trigger,
    );

    // Probe GPU hardware acceleration for video transcoding.
    let hw_accel = Arc::new(transcode::gpu_probe::probe_hwaccel(
        config.transcode.gpu_enabled,
        &config.transcode.gpu_device,
    ));

    // Loud, unambiguous banner so operators can see at a glance which
    // encoder will be used for video conversion. `probe_hwaccel` already
    // emits an info-level line, but it gets buried in the startup torrent.
    if hw_accel.is_gpu() {
        tracing::info!(
            "═══ Video transcode: GPU acceleration ACTIVE ({} → {}) ═══",
            hw_accel.accel_type,
            hw_accel.video_encoder,
        );
    } else {
        tracing::warn!("═══ Video transcode: CPU-only (libx264). No GPU encoder detected. ═══");
    }

    // Initialize global GPU config for the conversion pipeline.
    conversion::init_gpu_config((*hw_accel).clone(), config.transcode.gpu_fallback_to_cpu);

    // Build shared application state — cloned (via Arc) into every Axum handler.
    let state = AppState {
        pool,
        read_pool,
        config: Arc::new(config.clone()),
        rate_limiters,
        storage_root: storage_root_swap,
        scan_lock,
        audit_tx,
        storage_available,
        hw_accel,
        ai_active,
        geo_active,
        geo_dataset_available,
        geo_dataset_downloading,
        geo_trigger,
    };

    let mut app = Router::new()
        .route("/health", get(health::handlers::health))
        .route("/api/discover/info", get(health::handlers::discover_info))
        .nest("/api", routes::api_routes(state.clone()))
        // Security headers on all responses
        .layer(axum::middleware::from_fn(security::security_headers))
        // Disable Axum's default 2 MiB body limit — we rely on tower-http's
        // RequestBodyLimitLayer (configured from max_blob_size_bytes) instead.
        // Without this, the Bytes extractor rejects any upload > 2 MiB with a
        // plain-text 413 that the frontend can't parse as JSON.
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(
            config.storage.max_blob_size_bytes as usize,
        ))
        // Wide-open CORS — safe because the API uses stateless JWT auth, not
        // cookies. If cookie-based auth is ever added, restrict origins.
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
                // Expose custom response headers (e.g. X-Blob-Format) to
                // cross-origin browser clients. Safe under stateless JWT auth
                // with no cookies.
                .expose_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        // ── HTTP response compression (gzip + brotli) ────────────────────
        // Compresses JSON API responses, HTML, and other text-based content.
        // Binary blob endpoints explicitly set `Content-Encoding: identity`
        // to bypass this layer — encrypted bytes are incompressible and the
        // compression attempt itself is CPU-expensive.
        // Placed outermost so it wraps all responses after other middleware.
        .layer(CompressionLayer::new().gzip(true).br(true))
        .with_state(state);

    // Serve static web frontend if configured
    if !config.web.static_root.is_empty() {
        let static_path = std::path::PathBuf::from(&config.web.static_root);
        if static_path.exists() {
            tracing::info!("Serving web frontend from {:?}", static_path);
            app = app.fallback_service(tower_http::services::ServeDir::new(&static_path).fallback(
                tower_http::services::ServeFile::new(static_path.join("index.html")),
            ));
        } else {
            tracing::warn!(
                "Web static root {:?} does not exist, skipping static file serving",
                static_path
            );
        }
    }

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;

    if config.tls.enabled {
        let cert_path = config
            .tls
            .cert_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("TLS enabled but cert_path not set"))?;
        let key_path = config
            .tls
            .key_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("TLS enabled but key_path not set"))?;

        tracing::info!(
            "TLS enabled — loading cert from {:?}, key from {:?}",
            cert_path,
            key_path
        );

        let rustls_config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to load TLS config: {e}"))?;

        // ── HTTP → HTTPS redirect listener ─────────────────────────────
        // When TLS is enabled, every plain-HTTP request is upgraded to
        // HTTPS via a 301.  This guarantees that links / clients which
        // still default to `http://` end up on the encrypted endpoint
        // without surfacing a "connection refused" error.  Disable by
        // setting `[tls] redirect_http = false` (e.g. behind a reverse
        // proxy that already handles the upgrade).
        if config.tls.redirect_http {
            let redirect_port = config.tls.http_redirect_port;
            let https_port = config.server.port;
            let redirect_addr: SocketAddr = format!("0.0.0.0:{redirect_port}").parse()?;
            tokio::spawn(async move {
                spawn_https_redirect(redirect_addr, https_port).await;
            });
        }

        tracing::info!("Server ready (HTTPS)");
        axum_server::bind_rustls(addr, rustls_config)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!("Server ready");
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
    }

    Ok(())
}

/// Bind a plain-HTTP listener that 301-redirects every incoming request
/// to its HTTPS equivalent. Bind failures (e.g. EACCES on port 80 when
/// running unprivileged, or the port already being in use) are logged as
/// warnings but never abort startup — the HTTPS listener keeps serving.
async fn spawn_https_redirect(addr: SocketAddr, https_port: u16) {
    use axum::http::{header, HeaderMap, StatusCode, Uri};
    use axum::response::Redirect;

    async fn redirect_handler(
        headers: HeaderMap,
        uri: Uri,
        axum::extract::State(https_port): axum::extract::State<u16>,
    ) -> Result<Redirect, StatusCode> {
        // Strip any explicit port from the Host header before re-attaching
        // the HTTPS port, otherwise we'd produce hosts like
        // `example.com:80:8443`.
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let bare_host = host.split(':').next().unwrap_or(host);
        let path_and_query = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        let target = if https_port == 443 {
            format!("https://{bare_host}{path_and_query}")
        } else {
            format!("https://{bare_host}:{https_port}{path_and_query}")
        };
        // 301 (permanent) is appropriate: the operator opted in to TLS,
        // so this redirect is intended to stay in place.
        Ok(Redirect::permanent(&target))
    }

    let app = axum::Router::new()
        .fallback(redirect_handler)
        .with_state(https_port);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(
                "HTTP→HTTPS redirect listener failed to bind on {} ({}). \
                 HTTPS continues to serve normally; set `[tls] redirect_http = false` \
                 to silence this warning, or run with elevated privileges to bind port 80.",
                addr,
                e
            );
            return;
        }
    };
    tracing::info!("HTTP → HTTPS redirect listener bound on {}", addr);
    if let Err(e) = axum::serve(listener, app).await {
        tracing::warn!("HTTP→HTTPS redirect listener exited: {}", e);
    }
}

/// Remount a previously-configured SMB share at boot. Pulls the encrypted
/// password from `config.toml` (`[storage.smb]`), decrypts it with the JWT
/// secret, and runs the same `mount.cifs` logic the wizard uses.
async fn remount_smb_on_boot(
    cfg: &crate::setup::smb::SmbStoredConfig,
    jwt_secret: &str,
) -> Result<(), String> {
    use crate::setup::smb;

    let mount_point = std::path::PathBuf::from(&cfg.mount_point);
    if smb::is_mounted(&mount_point).await {
        return Ok(()); // already mounted (e.g. by systemd / fstab)
    }

    let mut target = smb::parse_smb_input(&cfg.address)?
        .ok_or_else(|| "Stored SMB address is not parseable".to_string())?;
    if !cfg.username.is_empty() {
        target.username = Some(cfg.username.clone());
    }
    if !cfg.domain.is_empty() {
        target.domain = Some(cfg.domain.clone());
    }
    if !cfg.password_enc.is_empty() {
        let pw = smb::decrypt_password(&cfg.password_enc, jwt_secret)?;
        target.password = Some(pw);
    }

    let creds_dir = std::path::PathBuf::from("data/smb-creds");
    smb::mount_smb(&target, &mount_point, &creds_dir)
        .await
        .map(|_| ())
}
