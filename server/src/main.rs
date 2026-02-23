mod audit;
mod auth;
mod backup;
mod blobs;
mod config;
mod db;
mod downloads;
mod error;
mod health;
mod media;
mod photos;
mod ratelimit;
mod security;
mod setup;
mod state;
mod trash;

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

    // Spawn background task to purge expired trash items every hour
    {
        let pool_clone = pool.clone();
        let storage_root_clone = config.storage.root.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                trash::handlers::purge_expired_trash(&pool_clone, &storage_root_clone).await;
            }
        });
    }

    // Spawn background task for backup server syncing
    {
        let pool_clone = pool.clone();
        let storage_root_clone = config.storage.root.clone();
        tokio::spawn(async move {
            backup::handlers::background_sync_task(pool_clone, storage_root_clone).await;
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
        .route("/admin/users", post(setup::admin::create_user))
        .route("/admin/users", get(setup::admin::list_users))
        .route("/admin/storage", get(setup::storage::get_storage))
        .route("/admin/storage", put(setup::storage::update_storage))
        .route("/admin/browse", get(setup::storage::browse_directory))
        // Server port configuration (admin only)
        .route("/admin/port", get(setup::port::get_port))
        .route("/admin/port", put(setup::port::update_port))
        .route("/admin/restart", post(setup::port::restart_server))
        // Server-side import — scan directories and serve raw files for client-side encryption
        .route("/admin/import/scan", get(setup::import::import_scan))
        .route("/admin/import/file", get(setup::import::import_file))
        // Plain-mode photos — list, serve, register, thumbnail
        .route("/photos", get(photos::handlers::list_photos))
        .route("/photos/register", post(photos::handlers::register_photo))
        .route("/photos/{id}/file", get(photos::handlers::serve_photo))
        .route("/photos/{id}/thumb", get(photos::handlers::serve_thumbnail))
        // Delete now soft-deletes to trash (30-day retention)
        .route("/photos/{id}", delete(trash::handlers::soft_delete_photo))
        // Plain-mode scan & register all files on disk
        .route("/admin/photos/scan", post(photos::handlers::scan_and_register))
        // Encryption settings
        .route("/settings/encryption", get(photos::encryption::get_encryption_settings))
        .route("/admin/encryption", put(photos::encryption::set_encryption_mode))
        .route("/admin/encryption/progress", post(photos::encryption::report_migration_progress))
        // Secure galleries
        .route("/galleries/secure", get(photos::galleries::list_secure_galleries))
        .route("/galleries/secure", post(photos::galleries::create_secure_gallery))
        .route("/galleries/secure/unlock", post(photos::galleries::unlock_secure_galleries))
        .route("/galleries/secure/{id}", delete(photos::galleries::delete_secure_gallery))
        .route("/galleries/secure/{id}/items", get(photos::galleries::list_gallery_items))
        .route("/galleries/secure/{id}/items", post(photos::galleries::add_gallery_item))
        // Trash — soft-deleted photos with 30-day retention
        .route("/trash", get(trash::handlers::list_trash))
        .route("/trash", delete(trash::handlers::empty_trash))
        .route("/trash/{id}", delete(trash::handlers::permanent_delete))
        .route("/trash/{id}/restore", post(trash::handlers::restore_from_trash))
        .route("/trash/{id}/thumb", get(trash::handlers::serve_trash_thumbnail))
        // Backup servers — admin only
        .route("/admin/backup/servers", get(backup::handlers::list_backup_servers))
        .route("/admin/backup/servers", post(backup::handlers::add_backup_server))
        .route("/admin/backup/servers/{id}", put(backup::handlers::update_backup_server))
        .route("/admin/backup/servers/{id}", delete(backup::handlers::remove_backup_server))
        .route("/admin/backup/servers/{id}/status", get(backup::handlers::check_backup_server_status))
        .route("/admin/backup/servers/{id}/logs", get(backup::handlers::get_sync_logs))
        .route("/admin/backup/servers/{id}/sync", post(backup::handlers::trigger_sync))
        .route("/admin/backup/servers/{id}/recover", post(backup::recovery::recover_from_backup))
        .route("/admin/backup/servers/{id}/photos", get(backup::recovery::proxy_backup_photos))
        .route("/admin/backup/discover", get(backup::handlers::discover_servers))
        // Backup serve — API-key authenticated, for server-to-server recovery
        .route("/backup/list", get(backup::serve::backup_list_photos))
        .route("/backup/download/{photo_id}", get(backup::serve::backup_download_photo))
        .route("/backup/download/{photo_id}/thumb", get(backup::serve::backup_download_thumb));

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
