mod audit;
mod auth;
mod blobs;
mod config;
mod db;
mod downloads;
mod error;
mod health;
mod ratelimit;
mod security;
mod setup;
mod state;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::ratelimit::RateLimiters;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = config::AppConfig::load()?;
    tracing::info!("Starting Simple Photos server v0.1.0");
    tracing::info!(
        "Listening on {}:{}",
        config.server.host,
        config.server.port
    );
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

    // Ensure storage directory exists (network mounts should already be present)
    tokio::fs::create_dir_all(&config.storage.root).await?;

    let pool = db::init_pool(&config.database).await?;

    // Initialize rate limiters and start background cleanup
    let rate_limiters = RateLimiters::new();
    rate_limiters.spawn_cleanup_task();

    // Spawn background task to clean up expired refresh tokens every hour
    {
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                let now = chrono::Utc::now().to_rfc3339();
                match sqlx::query(
                    "DELETE FROM refresh_tokens WHERE expires_at < ? OR (revoked = 1 AND created_at < ?)",
                )
                .bind(&now)
                .bind(&now)
                .execute(&pool_clone)
                .await
                {
                    Ok(result) => {
                        if result.rows_affected() > 0 {
                            tracing::info!(
                                "Cleaned up {} expired/revoked refresh tokens",
                                result.rows_affected()
                            );
                        }
                    }
                    Err(e) => tracing::error!("Failed to clean up tokens: {}", e),
                }

                // Also clean up old audit log entries (keep 90 days)
                let cutoff = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
                match sqlx::query("DELETE FROM audit_log WHERE created_at < ?")
                    .bind(&cutoff)
                    .execute(&pool_clone)
                    .await
                {
                    Ok(result) => {
                        if result.rows_affected() > 0 {
                            tracing::info!(
                                "Cleaned up {} old audit log entries (> 90 days)",
                                result.rows_affected()
                            );
                        }
                    }
                    Err(e) => tracing::error!("Failed to clean up audit log: {}", e),
                }
            }
        });
    }

    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        rate_limiters,
        storage_root: Arc::new(tokio::sync::RwLock::new(config.storage.root.clone())),
    };

    let api_routes = Router::new()
        // First-run setup (public)
        .route("/setup/status", get(setup::handlers::status))
        .route("/setup/init", post(setup::handlers::init))
        // Auth
        .route("/auth/register", post(auth::handlers::register))
        .route("/auth/login", post(auth::handlers::login))
        .route("/auth/login/totp", post(auth::handlers::login_totp))
        .route("/auth/refresh", post(auth::handlers::refresh))
        .route("/auth/logout", post(auth::handlers::logout))
        .route("/auth/password", put(auth::handlers::change_password))
        // 2FA
        .route("/auth/2fa/setup", post(auth::handlers::setup_2fa))
        .route("/auth/2fa/confirm", post(auth::handlers::confirm_2fa))
        .route("/auth/2fa/disable", post(auth::handlers::disable_2fa))
        // Blobs — photos, GIFs, videos, thumbnails, album manifests
        .route("/blobs", post(blobs::handlers::upload))
        .route("/blobs", get(blobs::handlers::list))
        .route("/blobs/{id}", get(blobs::handlers::download))
        .route("/blobs/{id}", delete(blobs::handlers::delete))
        // Downloads — Android APK, etc.
        .route("/downloads/android", get(downloads::handlers::android_apk))
        // Admin — user management, server config (requires admin role)
        .route("/admin/users", post(setup::handlers::create_user))
        .route("/admin/users", get(setup::handlers::list_users))
        .route("/admin/storage", get(setup::handlers::get_storage))
        .route("/admin/storage", put(setup::handlers::update_storage))
        .route("/admin/browse", get(setup::handlers::browse_directory));

    let mut app = Router::new()
        .route("/health", get(health::handlers::health))
        .nest("/api", api_routes)
        // Security headers on all responses
        .layer(axum::middleware::from_fn(security::security_headers))
        .layer(RequestBodyLimitLayer::new(
            config.storage.max_blob_size_bytes as usize,
        ))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Serve static web frontend if configured
    if !config.web.static_root.is_empty() {
        let static_path = std::path::PathBuf::from(&config.web.static_root);
        if static_path.exists() {
            tracing::info!("Serving web frontend from {:?}", static_path);
            app = app.fallback_service(
                tower_http::services::ServeDir::new(&static_path).fallback(
                    tower_http::services::ServeFile::new(static_path.join("index.html")),
                ),
            );
        } else {
            tracing::warn!(
                "Web static root {:?} does not exist, skipping static file serving",
                static_path
            );
        }
    }

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Server ready");
    axum::serve(listener, app).await?;

    Ok(())
}
