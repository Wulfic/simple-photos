//! Audit logging for security-relevant events.
//!
//! Stores a tamper-evident log of authentication events, data mutations,
//! and administrative actions. This is critical for incident response
//! and compliance.
//!
//! Events are stored in the `audit_log` table with:
//! - Timestamp (ISO 8601)
//! - Event type (login_success, login_failure, register, etc.)
//! - User ID (if known)
//! - IP address
//! - User-Agent
//! - Additional details (JSON)

use axum::http::HeaderMap;
use serde_json::Value as JsonValue;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::state::AppState;
use crate::state::AuditBroadcast;

/// All auditable event types.
#[derive(Debug, Clone, Copy)]
pub enum AuditEvent {
    // ── Authentication ───────────────────────────────────────────────
    /// Successful login
    LoginSuccess,
    /// Failed login (wrong password, user not found, etc.)
    LoginFailure,
    /// New user registration
    Register,
    /// Token refresh
    TokenRefresh,
    /// Logout
    Logout,
    /// 2FA setup initiated
    TotpSetup,
    /// 2FA confirmed/enabled
    TotpEnabled,
    /// 2FA disabled
    TotpDisabled,
    /// TOTP login success
    TotpLoginSuccess,
    /// TOTP login failure (wrong code)
    TotpLoginFailure,
    /// Backup code used for login
    BackupCodeUsed,
    /// Password changed
    PasswordChanged,
    /// Account locked out
    AccountLocked,
    /// Rate limit triggered.
    #[allow(dead_code)]
    RateLimited,

    // ── Blobs ────────────────────────────────────────────────────────
    /// Blob uploaded
    BlobUpload,
    /// Blob deleted
    BlobDelete,

    // ── Photos ───────────────────────────────────────────────────────
    /// Photo registered from disk
    PhotoRegister,
    /// Photo favorite toggled
    PhotoFavorite,
    /// Photo crop metadata updated
    PhotoCropSet,

    // ── Tags ─────────────────────────────────────────────────────────
    /// Tag added to a photo
    TagAdd,
    /// Tag removed from a photo
    TagRemove,

    // ── Trash ────────────────────────────────────────────────────────
    /// Item moved to trash (soft-delete)
    TrashSoftDelete,
    /// Item restored from trash
    TrashRestore,
    /// Item permanently deleted from trash
    TrashPermanentDelete,
    /// Entire trash emptied
    TrashEmpty,

    // ── Sharing ──────────────────────────────────────────────────────
    /// Shared album created
    SharedAlbumCreate,
    /// Shared album deleted
    SharedAlbumDelete,
    /// Member added to shared album
    SharedAlbumAddMember,
    /// Member removed from shared album
    SharedAlbumRemoveMember,
    /// Photo added to shared album
    SharedAlbumAddPhoto,
    /// Photo removed from shared album
    SharedAlbumRemovePhoto,

    // ── Backup Server Management ─────────────────────────────────────
    /// Backup server added
    BackupServerAdd,
    /// Backup server updated
    BackupServerUpdate,
    /// Backup server removed
    BackupServerRemove,

    // ── Backup Mode & Settings ───────────────────────────────────────
    /// Server mode changed (primary/backup)
    BackupModeChange,
    /// Audio backup setting toggled
    AudioBackupToggle,

    // ── Sync & Recovery ──────────────────────────────────────────────
    /// Manual sync triggered
    SyncTrigger,
    /// Force sync from primary requested (backup-side)
    SyncForceFromPrimary,
    /// Recovery from backup initiated
    RecoveryStart,

    // ── Background Tasks ─────────────────────────────────────────────
    /// Auto-scan completed
    AutoScanComplete,
    /// Trash purge completed (expired items)
    TrashPurgeComplete,
    /// Housekeeping completed (token/log cleanup)
    HousekeepingComplete,
    /// Encryption migration resumed/completed
    EncryptionMigrationComplete,
    /// Background backup sync cycle completed
    BackupSyncCycleComplete,

    // ── Admin ────────────────────────────────────────────────────────
    /// Admin action (e.g. config change, user management)
    AdminAction,
}

impl AuditEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            // Auth
            AuditEvent::LoginSuccess => "login_success",
            AuditEvent::LoginFailure => "login_failure",
            AuditEvent::Register => "register",
            AuditEvent::TokenRefresh => "token_refresh",
            AuditEvent::Logout => "logout",
            AuditEvent::TotpSetup => "totp_setup",
            AuditEvent::TotpEnabled => "totp_enabled",
            AuditEvent::TotpDisabled => "totp_disabled",
            AuditEvent::TotpLoginSuccess => "totp_login_success",
            AuditEvent::TotpLoginFailure => "totp_login_failure",
            AuditEvent::BackupCodeUsed => "backup_code_used",
            AuditEvent::PasswordChanged => "password_changed",
            AuditEvent::AccountLocked => "account_locked",
            AuditEvent::RateLimited => "rate_limited",
            // Blobs
            AuditEvent::BlobUpload => "blob_upload",
            AuditEvent::BlobDelete => "blob_delete",
            // Photos
            AuditEvent::PhotoRegister => "photo_register",
            AuditEvent::PhotoFavorite => "photo_favorite",
            AuditEvent::PhotoCropSet => "photo_crop_set",
            // Tags
            AuditEvent::TagAdd => "tag_add",
            AuditEvent::TagRemove => "tag_remove",
            // Trash
            AuditEvent::TrashSoftDelete => "trash_soft_delete",
            AuditEvent::TrashRestore => "trash_restore",
            AuditEvent::TrashPermanentDelete => "trash_permanent_delete",
            AuditEvent::TrashEmpty => "trash_empty",
            // Sharing
            AuditEvent::SharedAlbumCreate => "shared_album_create",
            AuditEvent::SharedAlbumDelete => "shared_album_delete",
            AuditEvent::SharedAlbumAddMember => "shared_album_add_member",
            AuditEvent::SharedAlbumRemoveMember => "shared_album_remove_member",
            AuditEvent::SharedAlbumAddPhoto => "shared_album_add_photo",
            AuditEvent::SharedAlbumRemovePhoto => "shared_album_remove_photo",
            // Backup management
            AuditEvent::BackupServerAdd => "backup_server_add",
            AuditEvent::BackupServerUpdate => "backup_server_update",
            AuditEvent::BackupServerRemove => "backup_server_remove",
            // Backup mode & settings
            AuditEvent::BackupModeChange => "backup_mode_change",
            AuditEvent::AudioBackupToggle => "audio_backup_toggle",
            // Sync & recovery
            AuditEvent::SyncTrigger => "sync_trigger",
            AuditEvent::SyncForceFromPrimary => "sync_force_from_primary",
            AuditEvent::RecoveryStart => "recovery_start",
            // Background tasks
            AuditEvent::AutoScanComplete => "auto_scan_complete",
            AuditEvent::TrashPurgeComplete => "trash_purge_complete",
            AuditEvent::HousekeepingComplete => "housekeeping_complete",
            AuditEvent::EncryptionMigrationComplete => "encryption_migration_complete",
            AuditEvent::BackupSyncCycleComplete => "backup_sync_cycle_complete",
            // Admin
            AuditEvent::AdminAction => "admin_action",
        }
    }
}

/// Write an audit log entry.  The actual database INSERT is spawned onto
/// the Tokio runtime so the calling handler returns **immediately** without
/// blocking on the audit write.  This is pure fire-and-forget — audit
/// logging should never slow down a user-facing request.
///
/// Reads `trust_proxy` from the app config to decide whether `X-Forwarded-For`
/// / `X-Real-IP` headers are trusted for IP extraction. This prevents spoofed
/// IPs from polluting audit logs on directly-exposed servers.
pub async fn log(
    state: &AppState,
    event: AuditEvent,
    user_id: Option<&str>,
    headers: &HeaderMap,
    details: Option<JsonValue>,
) {
    let trust_proxy = state.config.server.trust_proxy;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let ip = extract_ip(headers, trust_proxy);
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let details_str = details
        .map(|d| d.to_string())
        .unwrap_or_else(|| "{}".to_string());

    // Truncate user-agent to prevent DoS via huge headers
    let user_agent = if user_agent.len() > 512 {
        format!("{}…", &user_agent[..512])
    } else {
        user_agent
    };

    // Own the values that reference borrowed data so the spawned task is 'static.
    let pool = state.pool.clone();
    let audit_tx = state.audit_tx.clone();
    let event_str = event.as_str().to_string();
    let user_id_owned = user_id.map(|s| s.to_string());

    tokio::spawn(async move {
        let result = sqlx::query(
            "INSERT INTO audit_log (id, event_type, user_id, ip_address, user_agent, details, created_at, source_server) \
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(&id)
        .bind(&event_str)
        .bind(&user_id_owned)
        .bind(&ip)
        .bind(&user_agent)
        .bind(&details_str)
        .bind(&now)
        .execute(&pool)
        .await;

        if let Err(e) = result {
            tracing::error!(event = event_str.as_str(), error = %e, "Failed to write audit log");
        } else {
            // Broadcast to SSE subscribers — ignore send errors (no receivers = ok)
            let _ = audit_tx.send(AuditBroadcast {
                id: id.clone(),
                event_type: event_str.clone(),
                user_id: user_id_owned.clone(),
                ip_address: ip.clone(),
                user_agent: user_agent.clone(),
                details: details_str.clone(),
                created_at: now.clone(),
                source_server: None,
            });
        }
    });
}

/// Write an audit log entry for background tasks that have no HTTP headers.
/// Works directly with a pool reference instead of AppState.
/// Optionally broadcasts to the audit channel if a sender is provided.
pub fn log_background(
    pool: &SqlitePool,
    event: AuditEvent,
    details: Option<JsonValue>,
) {
    log_background_with_tx(pool, None, event, details);
}

/// Like `log_background` but with an optional broadcast sender for real-time delivery.
pub fn log_background_with_tx(
    pool: &SqlitePool,
    audit_tx: Option<&tokio::sync::broadcast::Sender<AuditBroadcast>>,
    event: AuditEvent,
    details: Option<JsonValue>,
) {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let details_str = details
        .map(|d| d.to_string())
        .unwrap_or_else(|| "{}".to_string());

    let pool = pool.clone();
    let event_str = event.as_str().to_string();
    let audit_tx = audit_tx.cloned();

    tokio::spawn(async move {
        let result = sqlx::query(
            "INSERT INTO audit_log (id, event_type, user_id, ip_address, user_agent, details, created_at, source_server) \
             VALUES (?, ?, NULL, 'background', 'system', ?, ?, NULL)",
        )
        .bind(&id)
        .bind(&event_str)
        .bind(&details_str)
        .bind(&now)
        .execute(&pool)
        .await;

        if let Err(e) = result {
            tracing::error!(event = event_str.as_str(), error = %e, "Failed to write audit log (background)");
        } else if let Some(tx) = audit_tx {
            let _ = tx.send(AuditBroadcast {
                id: id.clone(),
                event_type: event_str.clone(),
                user_id: None,
                ip_address: "background".to_string(),
                user_agent: "system".to_string(),
                details: details_str.clone(),
                created_at: now.clone(),
                source_server: None,
            });
        }
    });
}

/// Extract the client IP address from request headers.
///
/// When `trust_proxy` is `true`, checks `X-Forwarded-For` first (leftmost
/// entry = original client), then `X-Real-IP`. Returns `"unknown"` if
/// neither is present.
///
/// When `trust_proxy` is `false` (default), ignores proxy headers entirely
/// and returns `"direct"` — the server is directly exposed, so proxy
/// headers cannot be trusted and would let attackers poison audit logs.
///
/// # Security
/// Only set `trust_proxy = true` when behind a reverse proxy (nginx, Caddy)
/// that overwrites `X-Forwarded-For` / `X-Real-IP`. See also
/// [`crate::ratelimit::extract_client_ip`] which uses the same flag.
fn extract_ip(headers: &HeaderMap, trust_proxy: bool) -> String {
    if !trust_proxy {
        return "direct".to_string();
    }

    // X-Forwarded-For (first entry = original client)
    if let Some(xff) = headers.get("x-forwarded-for") {
        if let Ok(val) = xff.to_str() {
            if let Some(first) = val.split(',').next() {
                return first.trim().to_string();
            }
        }
    }
    // X-Real-IP
    if let Some(xri) = headers.get("x-real-ip") {
        if let Ok(val) = xri.to_str() {
            return val.trim().to_string();
        }
    }
    "unknown".to_string()
}
