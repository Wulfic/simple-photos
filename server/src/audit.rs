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
use uuid::Uuid;

use crate::state::AppState;

/// All auditable event types.
#[derive(Debug, Clone, Copy)]
pub enum AuditEvent {
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
    /// Blob uploaded
    BlobUpload,
    /// Blob deleted
    BlobDelete,
    /// Rate limit triggered.
    ///
    /// Currently emitted via `tracing::warn!` in [`crate::ratelimit::RateLimiter::check`]
    /// rather than here, because the rate limiter doesn't have access to `AppState`
    /// (would create a circular crate dependency). Kept as a variant so handlers
    /// can emit it explicitly for high-value audit trails if needed.
    #[allow(dead_code)]
    RateLimited,
    /// Account locked out
    AccountLocked,
    /// Admin action (e.g. config change, user management)
    AdminAction,
}

impl AuditEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
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
            AuditEvent::BlobUpload => "blob_upload",
            AuditEvent::BlobDelete => "blob_delete",
            AuditEvent::RateLimited => "rate_limited",
            AuditEvent::AccountLocked => "account_locked",
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
    let event_str = event.as_str().to_string();
    let user_id_owned = user_id.map(|s| s.to_string());

    tokio::spawn(async move {
        let result = sqlx::query(
            "INSERT INTO audit_log (id, event_type, user_id, ip_address, user_agent, details, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
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
            // Swallow errors — if the audit DB is broken, requests should still
            // succeed.  The tracing::error! ensures ops notice in server logs.
            tracing::error!(event = event_str.as_str(), error = %e, "Failed to write audit log");
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
