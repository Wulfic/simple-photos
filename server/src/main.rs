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
mod error;
mod export;
mod health;
mod http_utils;
mod import;
mod ingest;
mod media;
mod photos;
mod ratelimit;
mod routes;
mod sanitize;
mod security;
mod setup;
mod sharing;
mod state;
mod tags;
mod tasks;
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

    let (pool, read_pool) = db::init_pools(&config.database).await?;

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
    tasks::spawn_all(&pool, &config, &storage_root_swap, &scan_lock, &audit_tx, &storage_available);

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
    };

    let mut app = Router::new()
        .route("/health", get(health::handlers::health))
        .route("/api/discover/info", get(health::handlers::discover_info))
        .nest("/api", routes::api_routes())
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
                .allow_headers(Any),
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
                .map_err(|e| anyhow::anyhow!("Failed to load TLS config: {}", e))?;

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
