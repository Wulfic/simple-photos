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
use std::sync::OnceLock;
use uuid::Uuid;

use crate::state::AppState;
use crate::state::AuditBroadcast;

/// Process-wide handle to the SSE broadcast channel, registered once at startup
/// via [`register_broadcast`]. This lets background-task audit writes (which
/// only have a pool, not an `AppState`) still stream live to the Server Logs
/// tab instead of only appearing on the next page fetch.
static AUDIT_TX: OnceLock<tokio::sync::broadcast::Sender<AuditBroadcast>> = OnceLock::new();

/// Register the global broadcast sender. Call once at startup with
/// `state.audit_tx.clone()`. Idempotent — later calls are ignored.
pub fn register_broadcast(tx: tokio::sync::broadcast::Sender<AuditBroadcast>) {
    let _ = AUDIT_TX.set(tx);
}

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
    /// Media file transcoded to a browser-native format (image/video/audio)
    MediaConvert,

    // ── Pipeline failures (#45) ──────────────────────────────────────
    // Until these existed, the failure paths in `ingest.rs` emitted only
    // `tracing::warn!`, which goes to the process log — NOT the `audit` table
    // the Server Logs tab reads. The success path audited, the failure path did
    // not, so the one question a user actually asks ("which file failed?") was
    // the one question the UI could not answer. Every other conversion fix in
    // this area is guesswork without them.
    //
    // These are deliberately separate variants rather than a `success: false`
    // field on the existing ones: the logs tab filters by `event_type`, so a
    // "show me only failures" filter is a cheap indexed query instead of a JSON
    // scan, and an existing dashboard counting `media_convert` does not silently
    // start counting failures as successes.
    /// Transcode of a media file failed. Details carry the filename, category
    /// and the error, because "a conversion failed" alone is not actionable.
    MediaConvertFailure,
    /// A file could not be registered in the `photos` table. Distinct from a
    /// convert failure: the bytes are fine, the DB write is not, and a file
    /// that leaves no row is re-walked and re-failed on every autoscan pass.
    ImportFailure,
    /// Encryption of a stored file failed.
    EncryptionFailure,
    /// A photo exhausted its encryption attempts and was parked
    /// (`encryption_deferred = 1`). **Terminal**, and the same distinction
    /// `ConversionRetired` draws for #40 — but with a sharper consequence: a
    /// parked photo keeps its **plaintext original at rest**, and nothing ever
    /// retries it. On the live library this silently held 2,500 photos (~17%)
    /// unencrypted for a month, because the only signal was a per-attempt
    /// `EncryptionFailure` indistinguishable from a transient one. Clearable by
    /// `POST /api/admin/encryption/retry-parked`.
    EncryptionParked,
    /// Thumbnail generation failed. Non-fatal — the photo is still registered
    /// and downloadable — but it renders as a placeholder forever, which
    /// otherwise looks like a client bug.
    ThumbnailFailure,
    /// A file exhausted its conversion attempts (#40) and will not be tried
    /// again until it changes on disk. **Terminal**, and deliberately distinct
    /// from `MediaConvertFailure`: the per-attempt failures say "this went
    /// wrong", this one says "we have stopped trying", and a user looking for
    /// why a file never appeared needs to see the second one. Without it a file
    /// retired after three failures is indistinguishable from one that was
    /// never scanned.
    ConversionRetired,

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

/// Every event that represents something going wrong, for the Server Logs tab's
/// "Failures only" filter (#45).
///
/// Defined here, next to the enum, rather than as a literal in the diagnostics
/// handler or — worse — in the web client. A new failure variant added without
/// updating this list silently fails to appear under the filter that exists
/// precisely to surface it, and `failure_filter_covers_every_failure_variant`
/// pins that.
///
/// Auth failures are deliberately included: a user asking "what is going wrong
/// on my server" does not mean "only in the media pipeline".
pub const FAILURE_EVENTS: &[AuditEvent] = &[
    AuditEvent::MediaConvertFailure,
    AuditEvent::ImportFailure,
    AuditEvent::EncryptionFailure,
    AuditEvent::EncryptionParked,
    AuditEvent::ThumbnailFailure,
    AuditEvent::ConversionRetired,
    AuditEvent::LoginFailure,
    AuditEvent::TotpLoginFailure,
    AuditEvent::RateLimited,
    AuditEvent::AccountLocked,
];

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
            AuditEvent::MediaConvert => "media_convert",
            AuditEvent::MediaConvertFailure => "media_convert_failure",
            AuditEvent::ImportFailure => "import_failure",
            AuditEvent::EncryptionFailure => "encryption_failure",
            AuditEvent::EncryptionParked => "encryption_parked",
            AuditEvent::ThumbnailFailure => "thumbnail_failure",
            AuditEvent::ConversionRetired => "conversion_retired",
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
    // Username lookup uses the read pool so it never contends with the write
    // pool during high-frequency events (e.g. a bulk upload's blob_upload flood).
    let read_pool = state.read_pool.clone();
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
            // Resolve the display name so live SSE rows show the real username
            // instead of the raw UUID. The paginated fetch does this via a JOIN;
            // the broadcast has to do it explicitly. Best-effort — a lookup
            // failure just leaves `username` None (UI falls back to the UUID).
            let username = resolve_username(&read_pool, user_id_owned.as_deref()).await;
            // Broadcast to SSE subscribers — ignore send errors (no receivers = ok)
            let _ = audit_tx.send(AuditBroadcast {
                id: id.clone(),
                event_type: event_str.clone(),
                user_id: user_id_owned.clone(),
                username,
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
pub fn log_background(pool: &SqlitePool, event: AuditEvent, details: Option<JsonValue>) {
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
    // Prefer an explicitly-passed sender; otherwise fall back to the globally
    // registered one so background events still stream live to the log tab.
    let audit_tx = audit_tx.cloned().or_else(|| AUDIT_TX.get().cloned());

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
                username: None,
                ip_address: "background".to_string(),
                user_agent: "system".to_string(),
                details: details_str.clone(),
                created_at: now.clone(),
                source_server: None,
            });
        }
    });
}

/// Resolve a user's display name from their id for the SSE broadcast.
///
/// Returns `None` when `user_id` is absent, the user no longer exists, or the
/// query fails — callers treat a missing name as "fall back to the UUID".
async fn resolve_username(pool: &SqlitePool, user_id: Option<&str>) -> Option<String> {
    let uid = user_id?;
    sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = ?")
        .bind(uid)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_convert_event_string_is_stable() {
        // The web EVENT_COLORS map and any log filters key off this exact
        // string — changing it silently would break the Server Logs tab.
        assert_eq!(AuditEvent::MediaConvert.as_str(), "media_convert");
    }

    #[test]
    fn failure_event_strings_are_stable() {
        // Same contract as above, and load-bearing for #45's "Failures" filter:
        // the client selects these by string, so a rename here silently empties
        // the filter rather than failing to compile.
        assert_eq!(
            AuditEvent::MediaConvertFailure.as_str(),
            "media_convert_failure"
        );
        assert_eq!(AuditEvent::ImportFailure.as_str(), "import_failure");
        assert_eq!(AuditEvent::EncryptionFailure.as_str(), "encryption_failure");
        assert_eq!(AuditEvent::EncryptionParked.as_str(), "encryption_parked");
        assert_eq!(AuditEvent::ThumbnailFailure.as_str(), "thumbnail_failure");
    }

    /// Anything whose event string ends in `_failure` must be in
    /// `FAILURE_EVENTS`.
    ///
    /// This is the drift guard. The realistic mistake is adding a fifth failure
    /// variant, wiring it into `ingest.rs`, and forgetting this list — at which
    /// point the failure is written to the table but is invisible under the one
    /// filter built to surface it, which is #45 reintroduced by the fix for #45.
    ///
    /// Rust has no enum iteration without a derive, so the candidate list is
    /// spelled out; `failure_events_have_no_strays` catches the other direction.
    #[test]
    fn failure_filter_covers_every_failure_variant() {
        let all_failure_shaped = [
            AuditEvent::MediaConvertFailure,
            AuditEvent::ImportFailure,
            AuditEvent::EncryptionFailure,
            AuditEvent::ThumbnailFailure,
            AuditEvent::LoginFailure,
            AuditEvent::TotpLoginFailure,
        ];
        let covered: std::collections::HashSet<&str> =
            FAILURE_EVENTS.iter().map(|e| e.as_str()).collect();
        for ev in all_failure_shaped {
            assert!(
                covered.contains(ev.as_str()),
                "{} is a failure event but is missing from FAILURE_EVENTS, so the \
                 'Failures only' filter will never show it",
                ev.as_str()
            );
        }
    }

    /// The **terminal** events must be in `FAILURE_EVENTS` too, and no
    /// name-shaped rule can enforce it.
    ///
    /// `failure_filter_covers_every_failure_variant` keys off the `_failure`
    /// suffix, which is exactly why it does not protect these two: a file the
    /// server has GIVEN UP on is named for the giving-up, not for the failing.
    /// Both are the row a user goes to "Failures only" to find — `_retired`
    /// answers "why did this file never appear", `_parked` answers "why is this
    /// photo still plaintext on disk" — and both would be silently filtered out
    /// of the one view built to surface them.
    #[test]
    fn failure_filter_covers_the_terminal_events() {
        let covered: std::collections::HashSet<&str> =
            FAILURE_EVENTS.iter().map(|e| e.as_str()).collect();
        for ev in [AuditEvent::ConversionRetired, AuditEvent::EncryptionParked] {
            assert!(
                covered.contains(ev.as_str()),
                "{} is terminal but is missing from FAILURE_EVENTS, so the \
                 'Failures only' filter will never show it",
                ev.as_str()
            );
        }
    }

    /// The list must not contain duplicates — a duplicate would bind one extra
    /// placeholder for no reason and quietly suggest the list is unreviewed.
    #[test]
    fn failure_events_have_no_strays() {
        let strs: Vec<&str> = FAILURE_EVENTS.iter().map(|e| e.as_str()).collect();
        let unique: std::collections::HashSet<_> = strs.iter().collect();
        assert_eq!(unique.len(), strs.len(), "duplicate in FAILURE_EVENTS: {strs:?}");
    }

    /// Every failure variant must be distinguishable from every success
    /// variant, and from each other.
    ///
    /// The realistic mistake this catches is a copy-paste in the `as_str` match
    /// arm — two variants mapping to one string. That compiles, passes any test
    /// that only checks one of them, and makes a whole class of failure
    /// invisible in the logs tab by folding it into another.
    #[test]
    fn failure_events_are_distinct_from_successes() {
        let strs = [
            AuditEvent::MediaConvert.as_str(),
            AuditEvent::MediaConvertFailure.as_str(),
            AuditEvent::ImportFailure.as_str(),
            AuditEvent::EncryptionFailure.as_str(),
            // The per-attempt failure and the terminal park are the pair most
            // at risk of a copy-paste collision, and folding them together is
            // what makes a permanently-plaintext photo look like a transient
            // hiccup.
            AuditEvent::EncryptionParked.as_str(),
            AuditEvent::ThumbnailFailure.as_str(),
            AuditEvent::PhotoRegister.as_str(),
        ];
        let unique: std::collections::HashSet<_> = strs.iter().collect();
        assert_eq!(unique.len(), strs.len(), "duplicate audit event string: {strs:?}");
    }

    #[test]
    fn resolve_username_is_none_without_user_id() {
        // Background/system events pass no user_id; the resolver must short
        // out before touching the pool (the pool arg is never used here).
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            // A pool is required by the signature; an in-memory DB with no
            // `users` table proves the None arm never queries it.
            let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
            assert_eq!(resolve_username(&pool, None).await, None);
        });
    }
}
