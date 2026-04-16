//! Centralized API route definitions.
//!
//! Each helper builds a [`Router`] fragment for one domain. [`api_routes()`]
//! merges them all under the `/api` prefix consumed by `main.rs`.

use axum::routing::{delete, get, patch, post, put};
use axum::Router;

use crate::state::AppState;

/// Assemble every API route into a single router (mounted at `/api` by main).
pub fn api_routes() -> Router<AppState> {
    Router::new()
        .merge(setup_routes())
        .merge(auth_routes())
        .merge(blob_routes())
        .merge(download_routes())
        .merge(admin_routes())
        .merge(import_routes())
        .merge(photo_routes())
        .merge(gallery_routes())
        .merge(trash_routes())
        .merge(backup_routes())
        .merge(sharing_routes())
        .merge(tag_routes())
        .merge(client_log_routes())
        .merge(diagnostics_routes())
        .merge(export_routes())
}

// ── Setup & first-run ────────────────────────────────────────────────

fn setup_routes() -> Router<AppState> {
    Router::new()
        .route("/setup/status", get(crate::setup::handlers::status))
        .route("/setup/init", post(crate::setup::handlers::init))
        .route("/setup/pair", post(crate::setup::pair::pair))
        .route("/setup/discover", get(crate::setup::discovery::discover))
        .route(
            "/setup/verify-backup",
            post(crate::setup::pair::verify_backup),
        )
}

// ── Authentication & 2FA ─────────────────────────────────────────────

fn auth_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(crate::auth::handlers::register))
        .route("/auth/login", post(crate::auth::handlers::login))
        .route("/auth/login/totp", post(crate::auth::handlers::login_totp))
        .route("/auth/refresh", post(crate::auth::handlers::refresh))
        .route("/auth/logout", post(crate::auth::handlers::logout))
        .route("/auth/password", put(crate::auth::handlers::change_password))
        .route(
            "/auth/verify-password",
            post(crate::auth::handlers::verify_password),
        )
        .route("/auth/2fa/status", get(crate::auth::handlers::get_2fa_status))
        .route("/auth/2fa/setup", post(crate::auth::handlers::setup_2fa))
        .route("/auth/2fa/confirm", post(crate::auth::handlers::confirm_2fa))
        .route("/auth/2fa/disable", post(crate::auth::handlers::disable_2fa))
}

// ── Encrypted blobs ──────────────────────────────────────────────────

fn blob_routes() -> Router<AppState> {
    Router::new()
        .route("/blobs", post(crate::blobs::handlers::upload))
        .route("/blobs", get(crate::blobs::handlers::list))
        .route("/blobs/{id}", get(crate::blobs::download::download))
        .route("/blobs/{id}", delete(crate::blobs::handlers::delete))
        .route("/blobs/{id}/thumb", get(crate::blobs::download::download_thumb))
        // Blob soft-delete to trash (encrypted mode)
        .route("/blobs/{id}/trash", post(crate::trash::operations::soft_delete_blob))
}

// ── Downloads (public, unauthenticated) ──────────────────────────────

fn download_routes() -> Router<AppState> {
    Router::new().route(
        "/downloads/android",
        get(crate::downloads::handlers::android_apk),
    )
}

// ── Admin — user management, storage, port, SSL, import ──────────────

fn admin_routes() -> Router<AppState> {
    Router::new()
        // User management
        .route("/admin/users", post(crate::setup::admin::create_user))
        .route("/admin/users", get(crate::setup::admin::list_users))
        .route("/admin/users/{id}", delete(crate::setup::admin::delete_user))
        .route(
            "/admin/users/{id}/role",
            put(crate::setup::admin::update_user_role),
        )
        .route(
            "/admin/users/{id}/password",
            put(crate::setup::admin::admin_reset_password),
        )
        .route(
            "/admin/users/{id}/2fa",
            delete(crate::setup::admin_2fa::admin_reset_2fa),
        )
        .route(
            "/admin/users/{id}/2fa/setup",
            post(crate::setup::admin_2fa::admin_setup_2fa),
        )
        .route(
            "/admin/users/{id}/2fa/confirm",
            post(crate::setup::admin_2fa::admin_confirm_2fa),
        )
        // Storage
        .route("/admin/storage", get(crate::setup::storage::get_storage))
        .route("/admin/storage", put(crate::setup::storage::update_storage))
        .route("/admin/browse", get(crate::setup::storage::browse_directory))
        // Port
        .route("/admin/port", get(crate::setup::port::get_port))
        .route("/admin/port", put(crate::setup::port::update_port))
        .route("/admin/restart", post(crate::setup::port::restart_server))
        // SSL/TLS
        .route("/admin/ssl", get(crate::setup::ssl::get_ssl))
        .route("/admin/ssl", put(crate::setup::ssl::update_ssl))
        // Server-side import
        .route("/admin/import/scan", get(crate::setup::import::import_scan))
        .route("/admin/import/file", get(crate::setup::import::import_file))
}

// ── Google Photos / metadata import ──────────────────────────────────

fn import_routes() -> Router<AppState> {
    Router::new()
        .route("/import/metadata", post(crate::import::handlers::import_metadata))
        .route(
            "/import/metadata/batch",
            post(crate::import::handlers::batch_import_metadata),
        )
        .route(
            "/import/metadata/upload",
            post(crate::import::handlers::upload_sidecar),
        )
        .route(
            "/admin/import/google-photos/scan",
            get(crate::import::takeout::scan_takeout),
        )
        .route(
            "/admin/import/google-photos",
            post(crate::import::takeout::import_takeout),
        )
        .route(
            "/photos/{id}/metadata",
            get(crate::import::handlers::get_photo_metadata),
        )
        .route(
            "/photos/{id}/metadata",
            delete(crate::import::handlers::delete_photo_metadata),
        )
}

// ── Photos — list, serve, upload, scan, copy, render ─────────────────

fn photo_routes() -> Router<AppState> {
    Router::new()
        .route("/photos", get(crate::photos::handlers::list_photos))
        .route("/photos/encrypted-sync", get(crate::photos::sync::encrypted_sync))
        .route("/photos/register", post(crate::photos::handlers::register_photo))
        .route("/photos/upload", post(crate::photos::upload::upload_photo))
        .route("/photos/{id}/file", get(crate::photos::serve::serve_photo))
        .route("/photos/{id}/source-file", get(crate::photos::serve::serve_source_file))
        .route("/photos/{id}/thumb", get(crate::photos::serve::serve_thumbnail))
        .route(
            "/photos/{id}/thumbnail",
            get(crate::photos::serve::serve_thumbnail),
        )
        .route("/photos/{id}/web", get(crate::photos::serve::serve_web))
        .route(
            "/photos/{id}/favorite",
            put(crate::photos::handlers::toggle_favorite),
        )
        .route("/photos/{id}/crop", put(crate::editing::save::set_crop))
        .route("/photos/dimensions", patch(crate::photos::handlers::batch_update_dimensions))
        // Photo soft-delete to trash
        .route("/photos/{id}", delete(crate::trash::operations::soft_delete_photo))
        // Edit copies
        .route(
            "/photos/{id}/copies",
            post(crate::editing::edit_copies::create_edit_copy),
        )
        .route("/photos/{id}/copies", get(crate::editing::edit_copies::list_edit_copies))
        .route(
            "/photos/{id}/copies/{copy_id}",
            delete(crate::editing::edit_copies::delete_edit_copy),
        )
        // Duplicate photo (save as rendered copy)
        .route(
            "/photos/{id}/duplicate",
            post(crate::editing::save_copy::duplicate_photo),
        )
        // Render with baked-in edits (on-demand download)
        .route("/photos/{id}/render", post(crate::editing::render_download::render_photo))
        // Scan & register
        .route("/admin/photos/scan", post(crate::photos::scan::scan_and_register))
        // Conversion progress
        .route("/admin/conversion-status", get(crate::conversion::conversion_status))
        // Encryption key storage
        .route(
            "/admin/encryption/store-key",
            post(crate::photos::encryption::store_encryption_key),
        )
        // Storage stats
        .route(
            "/settings/storage-stats",
            get(crate::photos::storage_stats::get_storage_stats),
        )
}

// ── Secure galleries ─────────────────────────────────────────────────

fn gallery_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/galleries/secure",
            get(crate::photos::galleries::list_secure_galleries),
        )
        .route(
            "/galleries/secure",
            post(crate::photos::galleries::create_secure_gallery),
        )
        .route(
            "/galleries/secure/unlock",
            post(crate::photos::galleries::unlock_secure_galleries),
        )
        .route(
            "/galleries/secure/blob-ids",
            get(crate::photos::galleries::list_secure_blob_ids),
        )
        .route(
            "/galleries/secure/{id}",
            delete(crate::photos::galleries::delete_secure_gallery),
        )
        .route(
            "/galleries/secure/{id}/items",
            get(crate::photos::galleries::list_gallery_items),
        )
        .route(
            "/galleries/secure/{id}/items",
            post(crate::photos::galleries::add_gallery_item),
        )
}

// ── Trash ────────────────────────────────────────────────────────────

fn trash_routes() -> Router<AppState> {
    Router::new()
        .route("/trash", get(crate::trash::handlers::list_trash))
        .route("/trash", delete(crate::trash::operations::empty_trash))
        .route("/trash/{id}", delete(crate::trash::operations::permanent_delete))
        .route(
            "/trash/{id}/restore",
            post(crate::trash::operations::restore_from_trash),
        )
        .route(
            "/trash/{id}/thumb",
            get(crate::trash::handlers::serve_trash_thumbnail),
        )
}

// ── Backup — server management, sync, serve, recovery ────────────────

fn backup_routes() -> Router<AppState> {
    Router::new()
        // Admin backup management
        .route(
            "/admin/backup/servers",
            get(crate::backup::handlers::list_backup_servers),
        )
        .route(
            "/admin/backup/servers",
            post(crate::backup::handlers::add_backup_server),
        )
        .route(
            "/admin/backup/servers/{id}",
            put(crate::backup::handlers::update_backup_server),
        )
        .route(
            "/admin/backup/servers/{id}",
            delete(crate::backup::handlers::remove_backup_server),
        )
        .route(
            "/admin/backup/servers/{id}/status",
            get(crate::backup::handlers::check_backup_server_status),
        )
        .route(
            "/admin/backup/servers/{id}/logs",
            get(crate::backup::mode::get_sync_logs),
        )
        .route(
            "/admin/backup/servers/{id}/sync",
            post(crate::backup::sync::trigger_sync),
        )
        .route(
            "/admin/backup/servers/{id}/recover",
            post(crate::backup::recovery::recover_from_backup),
        )
        .route(
            "/admin/backup/servers/{id}/photos",
            get(crate::backup::proxy::proxy_backup_photos),
        )
        .route(
            "/admin/backup/servers/{id}/photos/{photo_id}/thumb",
            get(crate::backup::proxy::proxy_backup_thumbnail),
        )
        .route(
            "/admin/backup/servers/{id}/diagnostics",
            get(crate::backup::diagnostics::get_backup_diagnostics),
        )
        .route(
            "/admin/backup/discover",
            get(crate::backup::discover::discover_servers),
        )
        .route("/admin/backup/mode", get(crate::backup::mode::get_backup_mode))
        .route("/admin/backup/mode", post(crate::backup::mode::set_backup_mode))
        // Audio backup setting
        .route(
            "/settings/audio-backup",
            get(crate::backup::mode::get_audio_backup_setting),
        )
        .route(
            "/admin/audio-backup",
            put(crate::backup::mode::set_audio_backup_setting),
        )
        // Auto-scan trigger
        .route(
            "/admin/photos/auto-scan",
            post(crate::backup::autoscan::trigger_auto_scan),
        )
        // Server-to-server backup endpoints (API-key auth)
        .route("/backup/list", get(crate::backup::serve::backup_list_photos))
        .route("/backup/list-trash", get(crate::backup::serve::backup_list_trash))
        .route("/backup/list-users", get(crate::backup::serve_users::backup_list_users))
        .route(
            "/backup/list-users-full",
            get(crate::backup::serve_users::backup_list_users_full),
        )
        .route(
            "/backup/upsert-user",
            post(crate::backup::serve_users::backup_upsert_user),
        )
        .route("/backup/receive", post(crate::backup::serve_receive::backup_receive))
        .route("/backup/sync-deletions", post(crate::backup::serve::backup_sync_deletions))
        .route("/backup/sync-user-deletions",
            post(crate::backup::serve_users::backup_sync_user_deletions),
        )
        .route(
            "/backup/sync-secure-galleries",
            post(crate::backup::serve::backup_sync_secure_galleries),
        )
        .route("/backup/list-blobs", get(crate::backup::serve::backup_list_blobs))
        .route(
            "/backup/receive-blob",
            post(crate::backup::serve::backup_receive_blob),
        )
        .route(
            "/backup/sync-metadata",
            post(crate::backup::serve::backup_sync_metadata),
        )
        .route(
            "/backup/download/{photo_id}",
            get(crate::backup::serve::backup_download_photo),
        )
        .route(
            "/backup/download/{photo_id}/thumb",
            get(crate::backup::serve::backup_download_thumb),
        )
        .route(
            "/backup/request-sync",
            post(crate::backup::sync::handle_request_sync),
        )
        .route(
            "/admin/backup/force-sync",
            post(crate::backup::sync::force_sync_from_primary),
        )
        .route(
            "/backup/push-to",
            post(crate::backup::recovery::push_sync_to_target),
        )
        .route(
            "/backup/recovery-callback",
            post(crate::backup::recovery::recovery_callback),
        )
        .route(
            "/backup/report",
            post(crate::backup::diagnostics::receive_backup_report),
        )
        .route(
            "/backup/forward-logs",
            post(crate::backup::diagnostics::receive_forwarded_logs),
        )
}

// ── Shared albums ────────────────────────────────────────────────────

fn sharing_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/sharing/albums",
            get(crate::sharing::handlers::list_shared_albums),
        )
        .route(
            "/sharing/albums",
            post(crate::sharing::handlers::create_shared_album),
        )
        .route(
            "/sharing/albums/{id}",
            delete(crate::sharing::handlers::delete_shared_album),
        )
        .route(
            "/sharing/albums/{id}/members",
            get(crate::sharing::handlers::list_members),
        )
        .route(
            "/sharing/albums/{id}/members",
            post(crate::sharing::handlers::add_member),
        )
        .route(
            "/sharing/albums/{id}/members/{user_id}",
            delete(crate::sharing::handlers::remove_member),
        )
        .route(
            "/sharing/albums/{id}/photos",
            get(crate::sharing::handlers::list_shared_photos),
        )
        .route(
            "/sharing/albums/{id}/photos",
            post(crate::sharing::handlers::add_photo),
        )
        .route(
            "/sharing/albums/{album_id}/photos/{photo_id}",
            delete(crate::sharing::handlers::remove_photo),
        )
        .route(
            "/sharing/users",
            get(crate::sharing::handlers::list_users_for_sharing),
        )
}

// ── Tags & search ────────────────────────────────────────────────────

fn tag_routes() -> Router<AppState> {
    Router::new()
        .route("/tags", get(crate::tags::handlers::list_tags))
        .route("/photos/{id}/tags", get(crate::tags::handlers::get_photo_tags))
        .route("/photos/{id}/tags", post(crate::tags::handlers::add_tag))
        .route("/photos/{id}/tags", delete(crate::tags::handlers::remove_tag))
        .route("/search", get(crate::tags::handlers::search_photos))
}

// ── Client diagnostic logs ───────────────────────────────────────────

fn client_log_routes() -> Router<AppState> {
    Router::new()
        .route("/client-logs", post(crate::client_logs::handlers::submit_logs))
        .route("/admin/client-logs", get(crate::client_logs::handlers::list_logs))
}

// ── Server diagnostics & audit ───────────────────────────────────────

fn diagnostics_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/diagnostics",
            get(crate::diagnostics::handlers::get_diagnostics),
        )
        .route(
            "/admin/diagnostics/config",
            get(crate::diagnostics::handlers::get_diagnostics_config),
        )
        .route(
            "/admin/diagnostics/config",
            put(crate::diagnostics::handlers::update_diagnostics_config),
        )
        .route(
            "/admin/audit-logs",
            get(crate::diagnostics::handlers::list_audit_logs),
        )
        .route(
            "/admin/audit-logs/stream",
            get(crate::diagnostics::handlers::stream_audit_logs),
        )
        // External diagnostics (HTTP Basic Auth)
        .route(
            "/external/diagnostics",
            get(crate::diagnostics::external::external_full),
        )
        .route(
            "/external/diagnostics/health",
            get(crate::diagnostics::external::external_health),
        )
        .route(
            "/external/diagnostics/storage",
            get(crate::diagnostics::external::external_storage),
        )
        .route(
            "/external/diagnostics/audit",
            get(crate::diagnostics::external::external_audit),
        )
}

// ── Library export ───────────────────────────────────────────────────

fn export_routes() -> Router<AppState> {
    Router::new()
        .route("/export", post(crate::export::handlers::start_export))
        .route("/export/status", get(crate::export::handlers::export_status))
        .route("/export/files", get(crate::export::handlers::list_export_files))
        .route(
            "/export/files/{id}/download",
            get(crate::export::handlers::download_export_file),
        )
        .route(
            "/export/{job_id}",
            delete(crate::export::handlers::delete_export),
        )
}
