//! JWT and refresh-token helpers.
//!
//! JWTs use HS256 (HMAC-SHA256) for signing. Refresh tokens are stored as
//! SHA-256 hashes in the `refresh_tokens` table (the raw token is only
//! ever sent to the client).

use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

use super::models::Claims;

/// Create a signed HS256 JWT with the given user ID, role, and TTL.
pub fn create_jwt(
    user_id: &str,
    totp_required: bool,
    ttl_secs: u64,
    secret: &str,
    role: &str,
) -> Result<String, AppError> {
    let exp = (Utc::now().timestamp() as u64 + ttl_secs) as usize;
    let jti = Uuid::new_v4().to_string();
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        jti,
        totp_required,
        role: role.to_string(),
    };
    // Explicitly HS256 — prevent algorithm confusion attacks
    let header = Header::new(Algorithm::HS256);
    encode(
        &header,
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encoding error: {}", e)))
}

/// Issue a fresh access + refresh token pair for `user_id`.
///
/// The refresh token is stored as a SHA-256 hash in the `refresh_tokens` table.
/// The raw (unhashed) refresh token is returned to the caller for the client.
pub async fn issue_tokens(
    state: &AppState,
    user_id: &str,
) -> Result<(String, String), AppError> {
    // Fetch the user's role so it can be embedded in the JWT
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or_else(|| "user".to_string());

    let access_token = create_jwt(
        user_id,
        false,
        state.config.auth.access_token_ttl_secs,
        &state.config.auth.jwt_secret,
        &role,
    )?;

    let raw_refresh = Uuid::new_v4().to_string();
    let refresh_hash = hash_token(&raw_refresh);
    let expires_at = Utc::now()
        + chrono::Duration::days(state.config.auth.refresh_token_ttl_days as i64);

    sqlx::query(
        "INSERT INTO refresh_tokens (id, user_id, token_hash, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(&refresh_hash)
    .bind(expires_at.to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&state.pool)
    .await?;

    Ok((access_token, raw_refresh))
}

/// SHA-256 hash a raw token string for secure storage.
///
/// Refresh tokens are never stored in plaintext — only the hash is persisted,
/// so a database leak does not compromise active sessions.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
