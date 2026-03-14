//! Account lockout tracking — prevent brute-force attacks.

use axum::http::HeaderMap;
use chrono::Utc;

use crate::audit::{self, AuditEvent};
use crate::error::AppError;
use crate::state::AppState;

/// Maximum failed login attempts before account lockout.
pub const MAX_LOGIN_ATTEMPTS: i32 = 10;

/// Account lockout duration after exceeding max attempts (minutes).
pub const LOCKOUT_DURATION_MINS: i64 = 10;

/// Check if the account is currently locked out.
///
/// Returns `Err(Forbidden)` with a retry delay if locked. Automatically
/// resets expired lockouts.
pub async fn check_account_lockout(
    state: &AppState,
    user_id: &str,
) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, (i32, Option<String>)>(
        "SELECT failed_attempts, lockout_until FROM account_lockouts WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;

    if let Some((_attempts, lockout_until)) = row {
        if let Some(until) = lockout_until {
            if let Ok(lock_time) = chrono::DateTime::parse_from_rfc3339(&until) {
                let lock_time_utc = lock_time.with_timezone(&Utc);
                if Utc::now() < lock_time_utc {
                    let remaining = (lock_time_utc - Utc::now()).num_seconds();
                    return Err(AppError::Forbidden(format!(
                        "Account is temporarily locked. Try again in {} seconds.",
                        remaining.max(0)
                    )));
                }
                // Lockout expired — reset
                sqlx::query(
                    "UPDATE account_lockouts SET failed_attempts = 0, lockout_until = NULL WHERE user_id = ?",
                )
                .bind(user_id)
                .execute(&state.pool)
                .await?;
            }
        }
    }
    Ok(())
}

/// Increment the failed-login counter. Locks the account for
/// [`LOCKOUT_DURATION_MINS`] minutes if it reaches [`MAX_LOGIN_ATTEMPTS`].
pub async fn record_failed_login(
    state: &AppState,
    user_id: &str,
    headers: &HeaderMap,
) {
    let now = Utc::now().to_rfc3339();

    let result = sqlx::query(
        "INSERT INTO account_lockouts (user_id, failed_attempts, last_attempt_at) \
         VALUES (?, 1, ?) \
         ON CONFLICT(user_id) DO UPDATE SET \
           failed_attempts = failed_attempts + 1, \
           last_attempt_at = ?",
    )
    .bind(user_id)
    .bind(&now)
    .bind(&now)
    .execute(&state.pool)
    .await;

    if let Err(e) = result {
        tracing::error!("Failed to record failed login: {}", e);
        return;
    }

    let attempts: Option<i32> = sqlx::query_scalar(
        "SELECT failed_attempts FROM account_lockouts WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    if let Some(count) = attempts {
        if count >= MAX_LOGIN_ATTEMPTS {
            let lockout_until =
                (Utc::now() + chrono::Duration::minutes(LOCKOUT_DURATION_MINS)).to_rfc3339();

            // SECURITY: This write MUST succeed for brute-force protection
            // to work. Log loudly on failure so operators notice.
            if let Err(e) = sqlx::query(
                "UPDATE account_lockouts SET lockout_until = ? WHERE user_id = ?",
            )
            .bind(&lockout_until)
            .bind(user_id)
            .execute(&state.pool)
            .await
            {
                tracing::error!(
                    user_id = user_id,
                    error = %e,
                    "CRITICAL: Failed to set account lockout — brute-force protection bypassed"
                );
            }

            tracing::warn!(
                user_id = user_id,
                attempts = count,
                "Account locked after {} failed attempts",
                count
            );

            audit::log(
                state,
                AuditEvent::AccountLocked,
                Some(user_id),
                headers,
                Some(serde_json::json!({ "attempts": count })),
            )
            .await;
        }
    }
}

/// Deletes all failed-login / account-lockout records for the given user,
/// typically called after a successful authentication.
pub async fn clear_failed_logins(state: &AppState, user_id: &str) {
    if let Err(e) = sqlx::query("DELETE FROM account_lockouts WHERE user_id = ?")
        .bind(user_id)
        .execute(&state.pool)
        .await
    {
        // Non-critical: user stays "locked" longer than needed but can still
        // log in once the real lockout expires.
        tracing::warn!(user_id = user_id, error = %e, "Failed to clear account lockout record");
    }
}
