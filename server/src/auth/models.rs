use serde::{Deserialize, Serialize};

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

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user_id: String,
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

#[derive(Debug, Serialize)]
pub struct TotpRequiredResponse {
    pub requires_totp: bool,
    pub totp_session_token: String,
}

#[derive(Debug, Deserialize)]
pub struct TotpLoginRequest {
    pub totp_session_token: String,
    pub totp_code: Option<String>,
    pub backup_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct TotpConfirmRequest {
    pub totp_code: String,
}

#[derive(Debug, Deserialize)]
pub struct TotpDisableRequest {
    pub totp_code: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct TotpSetupResponse {
    pub otpauth_uri: String,
    pub backup_codes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    /// Unique JWT ID — enables per-token revocation
    #[serde(default)]
    pub jti: String,
    #[serde(default)]
    pub totp_required: bool,
}
