//! JWT and refresh-token helpers.

use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

use super::models::Claims;

pub fn create_jwt(
    user_id: &str,
    totp_required: bool,
    ttl_secs: u64,
    secret: &str,
) -> Result<String, AppError> {
    let exp = (Utc::now().timestamp() as u64 + ttl_secs) as usize;
    let jti = Uuid::new_v4().to_string();
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        jti,
        totp_required,
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

pub async fn issue_tokens(
    state: &AppState,
    user_id: &str,
) -> Result<(String, String), AppError> {
    let access_token = create_jwt(
        user_id,
        false,
        state.config.auth.access_token_ttl_secs,
        &state.config.auth.jwt_secret,
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

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
