mod audit;
mod auth;
mod backup;
mod blobs;
mod client_logs;
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
mod sharing;
mod state;
mod tags;
mod trash;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
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
    tracing::info!("Starting Simple Photos server v0.6.9");
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

                // Clean up old client diagnostic logs (keep 14 days)
                let client_log_cutoff = (chrono::Utc::now() - chrono::Duration::days(14)).to_rfc3339();
                match sqlx::query("DELETE FROM client_logs WHERE created_at < ?")
                    .bind(&client_log_cutoff)
                    .execute(&pool_clone)
                    .await
                {
                    Ok(result) => {
                        if result.rows_affected() > 0 {
                            tracing::info!(
                                "Cleaned up {} old client log entries (> 14 days)",
                                result.rows_affected()
                            );
                        }
                    }
                    Err(e) => tracing::error!("Failed to clean up client logs: {}", e),
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

    // Spawn background task for auto-scanning storage directory
    {
        let pool_clone = pool.clone();
        let storage_root_clone = config.storage.root.clone();
        let scan_interval = config.scan.auto_scan_interval_secs;
        tokio::spawn(async move {
            backup::handlers::background_auto_scan_task(pool_clone, storage_root_clone, scan_interval).await;
        });
    }

    // Spawn background task for backup-mode UDP broadcast
    {
        let pool_clone = pool.clone();
        let server_port = config.server.port;
        tokio::spawn(async move {
            backup::broadcast::background_broadcast_task(pool_clone, server_port).await;
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
        .route("/setup/pair", post(setup::handlers::pair))
        .route("/setup/discover", get(setup::handlers::discover))
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
        .route("/admin/users/{id}", delete(setup::admin::delete_user))
        .route("/admin/users/{id}/role", put(setup::admin::update_user_role))
        .route("/admin/users/{id}/password", put(setup::admin::admin_reset_password))
        .route("/admin/users/{id}/2fa", delete(setup::admin::admin_reset_2fa))
        .route("/admin/users/{id}/2fa/setup", post(setup::admin::admin_setup_2fa))
        .route("/admin/users/{id}/2fa/confirm", post(setup::admin::admin_confirm_2fa))
        .route("/admin/storage", get(setup::storage::get_storage))
        .route("/admin/storage", put(setup::storage::update_storage))
        .route("/admin/browse", get(setup::storage::browse_directory))
        // Server port configuration (admin only)
        .route("/admin/port", get(setup::port::get_port))
        .route("/admin/port", put(setup::port::update_port))
        .route("/admin/restart", post(setup::port::restart_server))
        // SSL/TLS configuration (admin only)
        .route("/admin/ssl", get(setup::ssl::get_ssl))
        .route("/admin/ssl", put(setup::ssl::update_ssl))
        .route("/admin/ssl/letsencrypt", post(setup::ssl::generate_letsencrypt))
        // Server-side import — scan directories and serve raw files for client-side encryption
        .route("/admin/import/scan", get(setup::import::import_scan))
        .route("/admin/import/file", get(setup::import::import_file))
        // Plain-mode photos — list, serve, register, thumbnail
        .route("/photos", get(photos::handlers::list_photos))
        .route("/photos/register", post(photos::handlers::register_photo))
        .route("/photos/upload", post(photos::handlers::upload_photo))
        .route("/photos/{id}/file", get(photos::handlers::serve_photo))
        .route("/photos/{id}/thumb", get(photos::handlers::serve_thumbnail))
        // Favorite toggle
        .route("/photos/{id}/favorite", put(photos::handlers::toggle_favorite))
        // Crop metadata
        .route("/photos/{id}/crop", put(photos::handlers::set_crop))
        // Delete now soft-deletes to trash (30-day retention)
        .route("/photos/{id}", delete(trash::handlers::soft_delete_photo))
        // Plain-mode scan & register all files on disk
        .route("/admin/photos/scan", post(photos::handlers::scan_and_register))
        // Encryption settings
        .route("/settings/encryption", get(photos::encryption::get_encryption_settings))
        .route("/admin/encryption", put(photos::encryption::set_encryption_mode))
        .route("/admin/encryption/progress", post(photos::encryption::report_migration_progress))
        .route("/photos/{id}/mark-encrypted", post(photos::encryption::mark_photo_encrypted))
        // Secure galleries
        .route("/galleries/secure", get(photos::galleries::list_secure_galleries))
        .route("/galleries/secure", post(photos::galleries::create_secure_gallery))
        .route("/galleries/secure/unlock", post(photos::galleries::unlock_secure_galleries))
        .route("/galleries/secure/blob-ids", get(photos::galleries::list_secure_blob_ids))
        .route("/galleries/secure/{id}", delete(photos::galleries::delete_secure_gallery))
        .route("/galleries/secure/{id}/items", get(photos::galleries::list_gallery_items))
        .route("/galleries/secure/{id}/items", post(photos::galleries::add_gallery_item))
        // Storage stats
        .route("/settings/storage-stats", get(photos::storage_stats::get_storage_stats))
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
        .route("/admin/backup/mode", get(backup::handlers::get_backup_mode))
        .route("/admin/backup/mode", post(backup::handlers::set_backup_mode))
        // Auto-scan trigger — called when web UI opens
        .route("/admin/photos/auto-scan", post(backup::handlers::trigger_auto_scan))
        // Backup serve — API-key authenticated, for server-to-server recovery
        .route("/backup/list", get(backup::serve::backup_list_photos))
        .route("/backup/receive", post(backup::serve::backup_receive))
        .route("/backup/download/{photo_id}", get(backup::serve::backup_download_photo))
        .route("/backup/download/{photo_id}/thumb", get(backup::serve::backup_download_thumb))
        // Shared albums — create, manage members, add/remove photos
        .route("/sharing/albums", get(sharing::handlers::list_shared_albums))
        .route("/sharing/albums", post(sharing::handlers::create_shared_album))
        .route("/sharing/albums/{id}", delete(sharing::handlers::delete_shared_album))
        .route("/sharing/albums/{id}/members", get(sharing::handlers::list_members))
        .route("/sharing/albums/{id}/members", post(sharing::handlers::add_member))
        .route("/sharing/albums/{id}/members/{user_id}", delete(sharing::handlers::remove_member))
        .route("/sharing/albums/{id}/photos", get(sharing::handlers::list_shared_photos))
        .route("/sharing/albums/{id}/photos", post(sharing::handlers::add_photo))
        .route("/sharing/albums/{album_id}/photos/{photo_id}", delete(sharing::handlers::remove_photo))
        .route("/sharing/users", get(sharing::handlers::list_users_for_sharing))
        // Tags — add, remove, list tags on photos; search by tag/filename
        .route("/tags", get(tags::handlers::list_tags))
        .route("/photos/{id}/tags", get(tags::handlers::get_photo_tags))
        .route("/photos/{id}/tags", post(tags::handlers::add_tag))
        .route("/photos/{id}/tags", delete(tags::handlers::remove_tag))
        .route("/search", get(tags::handlers::search_photos))
        // Client diagnostic logs — mobile clients submit backup debug logs
        .route("/client-logs", post(client_logs::handlers::submit_logs))
        .route("/admin/client-logs", get(client_logs::handlers::list_logs));

    let mut app = Router::new()
        .route("/health", get(health::handlers::health))
        .nest("/api", api_routes)
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

    if config.tls.enabled {
        let cert_path = config.tls.cert_path.as_deref()
            .ok_or_else(|| anyhow::anyhow!("TLS enabled but cert_path not set"))?;
        let key_path = config.tls.key_path.as_deref()
            .ok_or_else(|| anyhow::anyhow!("TLS enabled but key_path not set"))?;

        tracing::info!("TLS enabled — loading cert from {:?}, key from {:?}", cert_path, key_path);

        let rustls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(
            cert_path,
            key_path,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load TLS config: {}", e))?;

        tracing::info!("Server ready (HTTPS)");
        axum_server::bind_rustls(addr, rustls_config)
            .serve(app.into_make_service())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!("Server ready");
        axum::serve(listener, app).await?;
    }

    Ok(())
}
