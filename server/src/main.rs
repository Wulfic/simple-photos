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
//!    - Media format conversion (on-demand via Notify + 60 s poll)
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
mod crypto;
mod db;
mod diagnostics;
mod downloads;
mod error;
mod health;
mod import;
mod media;
mod photos;
mod ratelimit;
mod sanitize;
mod security;
mod setup;
mod sharing;
mod state;
mod tags;
mod trash;

/// Server version, read from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};
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
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = config::AppConfig::load()?;
    tracing::info!("Starting Simple Photos server v{VERSION}");
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

    // Ensure storage directory tree exists (network mounts should already be present)
    tokio::fs::create_dir_all(&config.storage.root).await?;
    // Create organized subdirectories under the storage root
    tokio::fs::create_dir_all(config.storage.root.join("blobs")).await?;
    tokio::fs::create_dir_all(config.storage.root.join("metadata")).await?;
    tokio::fs::create_dir_all(config.storage.root.join("logs/server")).await?;
    tokio::fs::create_dir_all(config.storage.root.join("logs/app")).await?;
    tracing::info!("Storage subdirectories initialized: blobs/, metadata/, logs/server/, logs/app/");

    let (pool, read_pool) = db::init_pools(&config.database).await?;

    // Initialize in-memory per-IP rate limiters for auth endpoints,
    // and start a background task that evicts stale entries every 5 min.
    let rate_limiters = RateLimiters::new();
    rate_limiters.spawn_cleanup_task();

    // Background housekeeping (runs every hour):
    // 1. Purge expired/revoked refresh tokens
    // 2. Delete audit log entries older than 90 days
    // 3. Delete client diagnostic logs older than 14 days
    //
    // All three DELETEs run inside a single transaction to reduce SQLite
    // WAL flushes from 3 → 1, cutting fsync overhead on each cycle.
    {
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                let now = chrono::Utc::now().to_rfc3339();
                let audit_cutoff = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
                let log_cutoff = (chrono::Utc::now() - chrono::Duration::days(14)).to_rfc3339();

                match pool_clone.begin().await {
                    Ok(mut tx) => {
                        // 1. Expired / revoked refresh tokens
                        match sqlx::query(
                            "DELETE FROM refresh_tokens WHERE expires_at < ? OR (revoked = 1 AND created_at < ?)",
                        )
                        .bind(&now)
                        .bind(&now)
                        .execute(&mut *tx)
                        .await
                        {
                            Ok(r) if r.rows_affected() > 0 => {
                                tracing::info!("Cleaned up {} expired/revoked refresh tokens", r.rows_affected());
                            }
                            Err(e) => tracing::error!("Failed to clean up tokens: {}", e),
                            _ => {}
                        }

                        // 2. Audit log entries older than 90 days
                        match sqlx::query("DELETE FROM audit_log WHERE created_at < ?")
                            .bind(&audit_cutoff)
                            .execute(&mut *tx)
                            .await
                        {
                            Ok(r) if r.rows_affected() > 0 => {
                                tracing::info!("Cleaned up {} old audit log entries (> 90 days)", r.rows_affected());
                            }
                            Err(e) => tracing::error!("Failed to clean up audit log: {}", e),
                            _ => {}
                        }

                        // 3. Client diagnostic logs older than 14 days
                        match sqlx::query("DELETE FROM client_logs WHERE created_at < ?")
                            .bind(&log_cutoff)
                            .execute(&mut *tx)
                            .await
                        {
                            Ok(r) if r.rows_affected() > 0 => {
                                tracing::info!("Cleaned up {} old client log entries (> 14 days)", r.rows_affected());
                            }
                            Err(e) => tracing::error!("Failed to clean up client logs: {}", e),
                            _ => {}
                        }

                        if let Err(e) = tx.commit().await {
                            tracing::error!("Housekeeping transaction commit failed: {}", e);
                        }
                    }
                    Err(e) => tracing::error!("Housekeeping: failed to begin transaction: {}", e),
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
            backup::sync::background_sync_task(pool_clone, storage_root_clone).await;
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

    // Spawn background task for media format conversion (MKV, AVI, HEIC, etc. → browser-native).
    // Converts non-web-native formats to JPEG/WebP/MP4 so they can play in the browser.
    // Wakes on-demand via `convert_notify` (e.g. after upload/scan) or polls every 60s.
    let convert_notify = Arc::new(tokio::sync::Notify::new());
    let conversion_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let encryption_key: Arc<tokio::sync::RwLock<Option<[u8; 32]>>> = Arc::new(tokio::sync::RwLock::new(None));
    let scan_lock: Arc<tokio::sync::Mutex<()>> = Arc::new(tokio::sync::Mutex::new(()));
    {
        let pool_clone = pool.clone();
        let read_pool_clone = read_pool.clone();
        let storage_root_clone = config.storage.root.clone();
        let notify_clone = convert_notify.clone();
        let active_clone = conversion_active.clone();
        let key_clone = encryption_key.clone();
        tokio::spawn(async move {
            photos::convert::background_convert_task(pool_clone, read_pool_clone, storage_root_clone, 60, notify_clone, active_clone, key_clone).await;
        });
    }

    // Spawn background task for auto-scanning storage directory.
    // Passes the ArcSwap<PathBuf> so the task always reads the *current*
    // storage root — respecting runtime changes via the setup wizard.
    let storage_root_swap = Arc::new(arc_swap::ArcSwap::from_pointee(config.storage.root.clone()));
    {
        let pool_clone = pool.clone();
        let storage_swap_clone = storage_root_swap.clone();
        let scan_interval = config.scan.auto_scan_interval_secs;
        let convert_notify_clone = convert_notify.clone();
        let scan_lock_clone = scan_lock.clone();
        tokio::spawn(async move {
            backup::autoscan::background_auto_scan_task(
                pool_clone,
                storage_swap_clone,
                scan_interval,
                convert_notify_clone,
                scan_lock_clone,
            ).await;
        });
    }

    // Build shared application state — cloned (via Arc) into every Axum handler.
    // Reuse the same ArcSwap that was passed to the background autoscan task
    // so runtime storage-path changes are visible everywhere atomically.
    let state = AppState {
        pool,
        read_pool,
        config: Arc::new(config.clone()),
        rate_limiters,
        storage_root: storage_root_swap,
        convert_notify,
        conversion_active,
        encryption_key,
        scan_lock,
    };

    let api_routes = Router::new()
        // First-run setup (public)
        .route("/setup/status", get(setup::handlers::status))
        .route("/setup/init", post(setup::handlers::init))
        .route("/setup/pair", post(setup::handlers::pair))
        .route("/setup/discover", get(setup::handlers::discover))
        .route("/setup/verify-backup", post(setup::handlers::verify_backup))
        // Auth
        .route("/auth/register", post(auth::handlers::register))
        .route("/auth/login", post(auth::handlers::login))
        .route("/auth/login/totp", post(auth::handlers::login_totp))
        .route("/auth/refresh", post(auth::handlers::refresh))
        .route("/auth/logout", post(auth::handlers::logout))
        .route("/auth/password", put(auth::handlers::change_password))
        .route("/auth/verify-password", post(auth::handlers::verify_password))
        // 2FA
        .route("/auth/2fa/status", get(auth::handlers::get_2fa_status))
        .route("/auth/2fa/setup", post(auth::handlers::setup_2fa))
        .route("/auth/2fa/confirm", post(auth::handlers::confirm_2fa))
        .route("/auth/2fa/disable", post(auth::handlers::disable_2fa))
        // Blobs — photos, GIFs, videos, thumbnails, album manifests
        .route("/blobs", post(blobs::handlers::upload))
        .route("/blobs", get(blobs::handlers::list))
        .route("/blobs/{id}", get(blobs::handlers::download))
        .route("/blobs/{id}", delete(blobs::handlers::delete))
        .route("/blobs/{id}/thumb", get(blobs::handlers::download_thumb))
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
        // Server-side import — scan directories and serve raw files for client-side encryption
        .route("/admin/import/scan", get(setup::import::import_scan))
        .route("/admin/import/file", get(setup::import::import_file))
        // Google Photos import — metadata parsing, Takeout directory scanning & import
        .route("/import/metadata", post(import::handlers::import_metadata))
        .route("/import/metadata/batch", post(import::handlers::batch_import_metadata))
        .route("/import/metadata/upload", post(import::handlers::upload_sidecar))
        .route("/admin/import/google-photos/scan", get(import::takeout::scan_takeout))
        .route("/admin/import/google-photos", post(import::takeout::import_takeout))
        .route("/photos/{id}/metadata", get(import::handlers::get_photo_metadata))
        .route("/photos/{id}/metadata", delete(import::handlers::delete_photo_metadata))
        // Photos — list, serve, register, thumbnail
        .route("/photos", get(photos::handlers::list_photos))
        .route("/photos/encrypted-sync", get(photos::sync::encrypted_sync))
        .route("/photos/register", post(photos::handlers::register_photo))
        .route("/photos/upload", post(photos::upload::upload_photo))
        .route("/photos/{id}/file", get(photos::handlers::serve_photo))
        .route("/photos/{id}/thumb", get(photos::handlers::serve_thumbnail))
        .route("/photos/{id}/web", get(photos::handlers::serve_web))
        // Favorite toggle
        .route("/photos/{id}/favorite", put(photos::handlers::toggle_favorite))
        // Crop metadata
        .route("/photos/{id}/crop", put(photos::handlers::set_crop))
        // Edit copies (Save Copy — metadata-only versions)
        .route("/photos/{id}/copies", post(photos::copies::create_edit_copy))
        .route("/photos/{id}/copies", get(photos::copies::list_edit_copies))
        .route("/photos/{id}/copies/{copy_id}", delete(photos::copies::delete_edit_copy))
        // Duplicate photo (Save Copy — creates a new photos row sharing the same file)
        .route("/photos/{id}/duplicate", post(photos::copies::duplicate_photo))
        // Delete now soft-deletes to trash (30-day retention)
        .route("/photos/{id}", delete(trash::handlers::soft_delete_photo))
        // Scan & register all files on disk
        .route("/admin/photos/scan", post(photos::scan::scan_and_register))
        // Trigger immediate background conversion pass
        .route("/admin/photos/convert", post(photos::convert::trigger_convert))
        // Supply encryption key and trigger re-conversion of encrypted blobs
        .route("/admin/photos/reconvert", post(photos::convert::trigger_reconvert))
        // Check conversion progress (pending items count) — available to any authenticated user
        .route("/photos/conversion-status", get(photos::convert::conversion_status))
        // Store encryption key so server-side operations can encrypt autonomously
        .route("/admin/encryption/store-key", post(photos::encryption::store_encryption_key))
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
        // Blob soft-delete to trash (encrypted mode)
        .route("/blobs/{id}/trash", post(trash::handlers::soft_delete_blob))
        // Backup servers — admin only
        .route("/admin/backup/servers", get(backup::handlers::list_backup_servers))
        .route("/admin/backup/servers", post(backup::handlers::add_backup_server))
        .route("/admin/backup/servers/{id}", put(backup::handlers::update_backup_server))
        .route("/admin/backup/servers/{id}", delete(backup::handlers::remove_backup_server))
        .route("/admin/backup/servers/{id}/status", get(backup::handlers::check_backup_server_status))
        .route("/admin/backup/servers/{id}/logs", get(backup::handlers::get_sync_logs))
        .route("/admin/backup/servers/{id}/sync", post(backup::sync::trigger_sync))
        .route("/admin/backup/servers/{id}/recover", post(backup::recovery::recover_from_backup))
        .route("/admin/backup/servers/{id}/photos", get(backup::recovery::proxy_backup_photos))
        .route("/admin/backup/discover", get(backup::handlers::discover_servers))
        .route("/admin/backup/mode", get(backup::handlers::get_backup_mode))
        .route("/admin/backup/mode", post(backup::handlers::set_backup_mode))
        // Audio backup setting
        .route("/settings/audio-backup", get(backup::handlers::get_audio_backup_setting))
        .route("/admin/audio-backup", put(backup::handlers::set_audio_backup_setting))
        // Auto-scan trigger — called when web UI opens
        .route("/admin/photos/auto-scan", post(backup::autoscan::trigger_auto_scan))
        // Backup serve — API-key authenticated, for server-to-server recovery
        .route("/backup/list", get(backup::serve::backup_list_photos))
        .route("/backup/list-trash", get(backup::serve::backup_list_trash))
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
        .route("/admin/client-logs", get(client_logs::handlers::list_logs))
        // Diagnostics — admin-only server metrics & audit log viewer
        .route("/admin/diagnostics", get(diagnostics::handlers::get_diagnostics))
        .route("/admin/diagnostics/config", get(diagnostics::handlers::get_diagnostics_config))
        .route("/admin/diagnostics/config", put(diagnostics::handlers::update_diagnostics_config))
        .route("/admin/audit-logs", get(diagnostics::handlers::list_audit_logs))
        // External diagnostics — HTTP Basic Auth, for server-to-server integration
        .route("/external/diagnostics", get(diagnostics::external::external_full))
        .route("/external/diagnostics/health", get(diagnostics::external::external_health))
        .route("/external/diagnostics/storage", get(diagnostics::external::external_storage))
        .route("/external/diagnostics/audit", get(diagnostics::external::external_audit));

    let mut app = Router::new()
        .route("/health", get(health::handlers::health))
        .route("/api/discover/info", get(health::handlers::discover_info))
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
        // Binary blob/photo/video downloads are already opaque encrypted bytes
        // or JPEG/MP4 (incompressible), so compression is a no-op on those.
        // Placed outermost so it wraps all responses after other middleware.
        .layer(CompressionLayer::new().gzip(true).br(true))
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
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!("Server ready");
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    }

    Ok(())
}
