//! TOTP and backup-code verification helpers.

use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::state::AppState;

/// Verify a 6-digit TOTP code against the user's Base32-encoded secret.
///
/// Uses SHA-1, 30-second step, and a single-step time window (±0 adjacent
/// periods).
pub fn verify_totp_code(
    secret_b32: &str,
    code: &str,
    // `_issuer` accepted for API compatibility but currently hardcoded to "SimplePhotos".
    _issuer: &str,
    account: &str,
) -> Result<(), AppError> {
    // Validate TOTP code format: must be exactly 6 digits
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::BadRequest(
            "TOTP code must be exactly 6 digits".into(),
        ));
    }

    let secret = totp_rs::Secret::Encoded(secret_b32.to_string());
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret
            .to_bytes()
            .map_err(|e| AppError::Internal(format!("TOTP secret error: {}", e)))?,
        Some("SimplePhotos".to_string()),
        account.to_string(),
    )
    .map_err(|e| AppError::Internal(format!("TOTP error: {}", e)))?;

    if totp
        .check_current(code)
        .map_err(|e| AppError::Internal(format!("TOTP time error: {}", e)))?
    {
        Ok(())
    } else {
        Err(AppError::Unauthorized("Invalid TOTP code".into()))
    }
}

/// Verify and consume a one-time backup code.
///
/// The code is SHA-256 hashed and compared against stored hashes. On success
/// the code is marked as used (single-use).
pub async fn verify_backup_code(
    state: &AppState,
    user_id: &str,
    backup_code: &str,
) -> Result<(), AppError> {
    let code_hash = hex::encode(Sha256::digest(backup_code.as_bytes()));

    let row = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM totp_backup_codes WHERE user_id = ? AND code_hash = ? AND used = 0",
    )
    .bind(user_id)
    .bind(&code_hash)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid or already used backup code".into()))?;

    sqlx::query("UPDATE totp_backup_codes SET used = 1 WHERE id = ?")
        .bind(&row.0)
        .execute(&state.pool)
        .await?;

    Ok(())
}
