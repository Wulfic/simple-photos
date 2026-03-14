//! Request/response DTOs and JWT claims for the auth subsystem.
//!
//! All request structs derive `Deserialize` (for JSON parsing by Axum),
//! and all response structs derive `Serialize` (for JSON output).

use serde::{Deserialize, Serialize};

/// Database row for the `users` table.
#[derive(Debug, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub created_at: String,
    pub storage_quota_bytes: i64,
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
}

/// Body for POST /api/auth/register.
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
}

/// Response for POST /api/auth/register.
#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user_id: String,
    pub username: String,
}

/// Body for POST /api/auth/login.
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Response for successful login (or refresh). Contains JWT + refresh token.
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

/// Body for POST /api/auth/login/totp — carries either a TOTP code or a backup code.
#[derive(Debug, Deserialize)]
pub struct TotpLoginRequest {
    pub totp_session_token: String,
    pub totp_code: Option<String>,
    pub backup_code: Option<String>,
}

/// Body for POST /api/auth/refresh.
#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Response for POST /api/auth/refresh — rotated access + refresh tokens.
#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

/// Body for POST /api/auth/logout.
#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

/// Body for POST /api/auth/2fa/confirm — user proves they have the TOTP app configured.
#[derive(Debug, Deserialize)]
pub struct TotpConfirmRequest {
    pub totp_code: String,
}

/// Body for POST /api/auth/2fa/disable — requires a valid TOTP code to turn off 2FA.
#[derive(Debug, Deserialize)]
pub struct TotpDisableRequest {
    pub totp_code: String,
}

/// Body for PUT /api/auth/password.
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Body for POST /api/auth/verify-password.
#[derive(Debug, Deserialize)]
pub struct VerifyPasswordRequest {
    pub password: String,
}

/// Response for POST /api/auth/2fa/setup — contains the otpauth URI for QR
/// display and single-use backup codes.
#[derive(Debug, Serialize)]
pub struct TotpSetupResponse {
    pub otpauth_uri: String,
    pub backup_codes: Vec<String>,
}

/// JWT claims payload, signed with HS256.
///
/// The `sub` field is the user ID. `totp_required` is `true` for TOTP session
/// tokens (half-authenticated); full tokens have it `false`.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    /// Unique JWT ID — currently used only for logging / tracing.
    /// No server-side revocation list is checked; JWTs are valid until
    /// expiry. To revoke access early, revoke the *refresh* token instead.
    #[serde(default)]
    pub jti: String,
    #[serde(default)]
    pub totp_required: bool,
    /// User role ("admin" or "user") — embedded so the frontend can gate UI
    #[serde(default)]
    pub role: String,
}
