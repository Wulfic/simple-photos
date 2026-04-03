//! Admin 2FA management: reset, setup, and confirm on behalf of users.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::auth::models::TotpSetupResponse;
use crate::error::AppError;
use crate::state::AppState;

use super::admin::require_admin;

// ── Admin reset 2FA ────────────────────────────────────────────────────────

/// Admin-only: Disable 2FA for a user.
///
/// DELETE /api/admin/users/{id}/2fa
pub async fn admin_reset_2fa(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    let result = sqlx::query("UPDATE users SET totp_enabled = 0, totp_secret = NULL WHERE id = ?")
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    // Delete backup codes for this user
    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "admin_reset_2fa",
            "target_user_id": user_id
        })),
    )
    .await;

    tracing::info!("Admin '{}' reset 2FA for user '{}'", auth.user_id, user_id);

    Ok(Json(serde_json::json!({
        "message": "Two-factor authentication disabled for user"
    })))
}

// ── Admin setup 2FA for a user ─────────────────────────────────────────────

/// Admin-only: Generate 2FA TOTP secret and backup codes for a target user.
///
/// POST /api/admin/users/{id}/2fa/setup
pub async fn admin_setup_2fa(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<Json<TotpSetupResponse>, AppError> {
    require_admin(&state, &auth).await?;

    let user = sqlx::query_as::<_, (String, bool)>(
        "SELECT username, totp_enabled != 0 FROM users WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let (username, totp_enabled) = user;
    if totp_enabled {
        return Err(AppError::BadRequest(
            "2FA is already enabled for this user".into(),
        ));
    }

    let secret = totp_rs::Secret::generate_secret();
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret
            .to_bytes()
            .map_err(|e| AppError::Internal(format!("TOTP secret error: {}", e)))?,
        Some("SimplePhotos".to_string()),
        username.clone(),
    )
    .map_err(|e| AppError::Internal(format!("TOTP creation error: {}", e)))?;

    let otpauth_uri = totp.get_url();
    let secret_b32 = secret.to_encoded().to_string();

    sqlx::query("UPDATE users SET totp_secret = ? WHERE id = ?")
        .bind(&secret_b32)
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    // Generate 10 backup codes
    let backup_codes: Vec<String> = {
        let mut rng = rand::thread_rng();
        (0..10)
            .map(|_| {
                (0..8)
                    .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
                    .collect()
            })
            .collect()
    };

    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    for code in &backup_codes {
        let code_hash = hex::encode(Sha256::digest(code.as_bytes()));
        sqlx::query("INSERT INTO totp_backup_codes (id, user_id, code_hash) VALUES (?, ?, ?)")
            .bind(Uuid::new_v4().to_string())
            .bind(&user_id)
            .bind(&code_hash)
            .execute(&state.pool)
            .await?;
    }

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "admin_setup_2fa",
            "target_user_id": user_id
        })),
    )
    .await;

    tracing::info!(
        "Admin '{}' initiated 2FA setup for user '{}'",
        auth.user_id,
        user_id
    );

    Ok(Json(TotpSetupResponse {
        otpauth_uri,
        backup_codes,
    }))
}

// ── Admin confirm 2FA for a user ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AdminConfirm2faRequest {
    pub totp_code: String,
}

/// Admin-only: Confirm 2FA for a target user after admin_setup_2fa.
///
/// POST /api/admin/users/{id}/2fa/confirm
pub async fn admin_confirm_2fa(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Json(req): Json<AdminConfirm2faRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    let user = sqlx::query_as::<_, (Option<String>, bool)>(
        "SELECT totp_secret, totp_enabled != 0 FROM users WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let (totp_secret, totp_enabled) = user;
    if totp_enabled {
        return Err(AppError::BadRequest(
            "2FA is already enabled for this user".into(),
        ));
    }

    let secret_b32 = totp_secret.ok_or(AppError::BadRequest(
        "2FA setup not initiated. Call admin setup first".into(),
    ))?;

    let secret = totp_rs::Secret::Encoded(secret_b32)
        .to_bytes()
        .map_err(|e| AppError::Internal(format!("TOTP decode error: {}", e)))?;

    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret,
        Some("SimplePhotos".to_string()),
        String::new(),
    )
    .map_err(|e| AppError::Internal(format!("TOTP creation error: {}", e)))?;

    if !totp
        .check_current(&req.totp_code)
        .map_err(|e| AppError::Internal(format!("TOTP error: {}", e)))?
    {
        return Err(AppError::BadRequest("Invalid TOTP code".into()));
    }

    sqlx::query("UPDATE users SET totp_enabled = 1 WHERE id = ?")
        .bind(&user_id)
        .execute(&state.pool)
        .await?;

    audit::log(
        &state,
        AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "admin_confirm_2fa",
            "target_user_id": user_id
        })),
    )
    .await;

    tracing::info!(
        "Admin '{}' confirmed 2FA for user '{}'",
        auth.user_id,
        user_id
    );

    Ok(Json(serde_json::json!({
        "message": "Two-factor authentication enabled for user"
    })))
}
