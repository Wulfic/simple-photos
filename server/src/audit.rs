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
    /// Rate limit triggered
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

/// Write an audit log entry. This is fire-and-forget — audit logging should
/// never cause a request to fail.
pub async fn log(
    pool: &SqlitePool,
    event: AuditEvent,
    user_id: Option<&str>,
    headers: &HeaderMap,
    details: Option<JsonValue>,
) {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let ip = extract_ip(headers);
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

    let result = sqlx::query(
        "INSERT INTO audit_log (id, event_type, user_id, ip_address, user_agent, details, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(event.as_str())
    .bind(user_id)
    .bind(&ip)
    .bind(&user_agent)
    .bind(&details_str)
    .bind(&now)
    .execute(pool)
    .await;

    if let Err(e) = result {
        // We intentionally swallow errors here — if the audit DB is broken, we
        // still want requests to succeed. The tracing::error! ensures ops notice
        // in server logs.
        tracing::error!(event = event.as_str(), error = %e, "Failed to write audit log");
    }
}

/// Extract the client IP address from proxy headers.
///
/// Checks `X-Forwarded-For` first (leftmost entry = original client),
/// then `X-Real-IP`. Returns `"unknown"` if neither is present.
///
/// # Security
/// These headers are only trustworthy when deployed behind a reverse proxy
/// (nginx, Caddy) that overwrites them. A direct client can forge these.
fn extract_ip(headers: &HeaderMap) -> String {
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
