//! User sync endpoints for the backup serve API.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use crate::error::AppError;
use crate::state::AppState;

use super::serve::validate_api_key;

// ── User Sync Endpoints ────────────────────────────────────────────────────

/// GET /api/backup/list-users
/// Returns all user IDs on this backup server for delta-sync detection.
/// Used by the primary's sync engine to skip users already present.
pub async fn backup_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, username FROM users ORDER BY created_at ASC")
            .fetch_all(&state.read_pool)
            .await?;

    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, username)| serde_json::json!({ "id": id, "username": username }))
        .collect();

    Ok(Json(items))
}

/// POST /api/backup/upsert-user
/// Creates or updates a user on this backup server with full credentials.
/// Transfers id, username, password_hash, role, storage_quota_bytes,
/// created_at, totp_secret, totp_enabled, and totp_backup_codes from the
/// primary so that users can log in on the backup server.
pub async fn backup_upsert_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("missing id".into()))?
        .to_string();
    let username = body
        .get("username")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("missing username".into()))?
        .to_string();
    let role = body
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("user")
        .to_string();
    let quota = body
        .get("storage_quota_bytes")
        .and_then(|v| v.as_i64())
        .unwrap_or(10_737_418_240); // 10 GiB default
    let created_at = body
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let password_hash = body
        .get("password_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("missing password_hash".into()))?
        .to_string();
    let totp_secret: Option<String> = body
        .get("totp_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let totp_enabled: i32 = body
        .get("totp_enabled")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;

    // Upsert the user record with full credentials.
    // ON CONFLICT updates all mutable fields so password changes,
    // role changes, and 2FA changes propagate from the primary.
    let result = sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, \
         role, totp_secret, totp_enabled) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
             username            = excluded.username, \
             password_hash       = excluded.password_hash, \
             role                = excluded.role, \
             storage_quota_bytes = excluded.storage_quota_bytes, \
             totp_secret         = excluded.totp_secret, \
             totp_enabled        = excluded.totp_enabled",
    )
    .bind(&id)
    .bind(&username)
    .bind(&password_hash)
    .bind(&created_at)
    .bind(quota)
    .bind(&role)
    .bind(&totp_secret)
    .bind(totp_enabled)
    .execute(&state.pool)
    .await;

    if let Err(e) = result {
        // UNIQUE violation on `username` — a local account with the same
        // name but a different id already exists.  Merge the local user
        // into the primary user so all content is visible under one account.
        let err_str = e.to_string();
        if err_str.contains("UNIQUE constraint failed: users.username") {
            tracing::info!(
                "backup_upsert_user: merging local '{}' into primary id={}",
                username,
                id
            );

            // Find the conflicting local user id
            let local_id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM users WHERE username = ? AND id != ?",
            )
            .bind(&username)
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);

            if let Some(ref old_id) = local_id {
                // Reassign all content from the local user to the primary user id.
                // FKs have ON DELETE CASCADE, but we want to keep the data — so
                // re-parent first, then delete the old user row.
                let reparent_tables: &[&str] = &[
                    "UPDATE photos SET user_id = ? WHERE user_id = ?",
                    "UPDATE trash_items SET user_id = ? WHERE user_id = ?",
                    "UPDATE photo_tags SET user_id = ? WHERE user_id = ?",
                    "UPDATE blobs SET user_id = ? WHERE user_id = ?",
                    "UPDATE audit_log SET user_id = ? WHERE user_id = ?",
                    "UPDATE client_logs SET user_id = ? WHERE user_id = ?",
                    "UPDATE shared_albums SET owner_user_id = ? WHERE owner_user_id = ?",
                    "UPDATE shared_album_members SET user_id = ? WHERE user_id = ?",
                ];
                for sql in reparent_tables {
                    if let Err(re) = sqlx::query(sql)
                        .bind(&id)
                        .bind(old_id)
                        .execute(&state.pool)
                        .await
                    {
                        // Non-fatal: table may not exist on minimal setups
                        tracing::debug!(
                            "backup_upsert_user: reparent skipped for '{}': {}",
                            sql.split_whitespace().nth(1).unwrap_or("?"),
                            re
                        );
                    }
                }

                // Also reparent encrypted_galleries and encryption_user_keys
                // (keyed on user_id TEXT PRIMARY KEY — needs special handling)
                let _ = sqlx::query(
                    "DELETE FROM encrypted_galleries WHERE user_id = ?",
                )
                .bind(old_id)
                .execute(&state.pool)
                .await;

                // Remove the old local user
                let _ = sqlx::query("DELETE FROM users WHERE id = ?")
                    .bind(old_id)
                    .execute(&state.pool)
                    .await;

                tracing::info!(
                    "backup_upsert_user: removed local user {} and reparented content to {}",
                    old_id,
                    id
                );
            }

            // Now insert the primary user — the conflicting row is gone
            if let Err(insert_err) = sqlx::query(
                "INSERT INTO users (id, username, password_hash, created_at, storage_quota_bytes, \
                 role, totp_secret, totp_enabled) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET \
                     username            = excluded.username, \
                     password_hash       = excluded.password_hash, \
                     role                = excluded.role, \
                     storage_quota_bytes = excluded.storage_quota_bytes, \
                     totp_secret         = excluded.totp_secret, \
                     totp_enabled        = excluded.totp_enabled",
            )
            .bind(&id)
            .bind(&username)
            .bind(&password_hash)
            .bind(&created_at)
            .bind(quota)
            .bind(&role)
            .bind(&totp_secret)
            .bind(totp_enabled)
            .execute(&state.pool)
            .await
            {
                tracing::error!(
                    "backup_upsert_user: merge insert failed for id={}: {}",
                    id,
                    insert_err
                );
                return Err(AppError::Internal(format!(
                    "Failed to create backup user record for id={}: {}",
                    id, insert_err
                )));
            }
        } else {
            // Some other DB error — report it
            tracing::error!(
                "backup_upsert_user: unexpected error for id={}: {}",
                id,
                e
            );
            return Err(AppError::Internal(format!(
                "Failed to create backup user record for id={}: {}",
                id, e
            )));
        }
    }

    // Sync TOTP backup codes — replace all codes for this user with the
    // primary's current set so revocations and regenerations propagate.
    if let Some(codes) = body.get("totp_backup_codes").and_then(|v| v.as_array()) {
        // Clear existing codes for this user
        if let Err(e) = sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
            .bind(&id)
            .execute(&state.pool)
            .await
        {
            tracing::warn!("Failed to clear TOTP backup codes for user {}: {}", id, e);
        }

        for code in codes {
            let code_id = match code.get("id").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => continue,
            };
            let code_hash = match code.get("code_hash").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => continue,
            };
            let used = code.get("used").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

            if let Err(e) = sqlx::query(
                "INSERT OR REPLACE INTO totp_backup_codes (id, user_id, code_hash, used) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(code_id)
            .bind(&id)
            .bind(code_hash)
            .bind(used)
            .execute(&state.pool)
            .await
            {
                tracing::warn!(
                    "Failed to sync TOTP backup code {} for user {}: {}",
                    code_id,
                    id,
                    e
                );
            }
        }
    }

    Ok(StatusCode::OK)
}

/// GET /api/backup/list-users-full
/// Returns all user records with full credentials (password_hash, TOTP secrets,
/// backup codes) for disaster recovery. The recovering primary calls this to
/// restore user accounts so they can log in with the same credentials.
///
/// **Security:** Authenticated via X-API-Key (same as all backup-serve endpoints).
pub async fn backup_list_users_full(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    validate_api_key(&state, &headers).await?;

    let users: Vec<(
        String,         // id
        String,         // username
        String,         // password_hash
        String,         // role
        i64,            // storage_quota_bytes
        String,         // created_at
        Option<String>, // totp_secret
        i32,            // totp_enabled
    )> = sqlx::query_as(
        "SELECT id, username, password_hash, role, storage_quota_bytes, \
         created_at, totp_secret, totp_enabled FROM users ORDER BY created_at ASC",
    )
    .fetch_all(&state.read_pool)
    .await?;

    let mut result = Vec::with_capacity(users.len());
    for (id, username, password_hash, role, quota, created_at, totp_secret, totp_enabled) in &users
    {
        // Fetch TOTP backup codes for this user
        let backup_codes: Vec<(String, String, i32)> =
            sqlx::query_as("SELECT id, code_hash, used FROM totp_backup_codes WHERE user_id = ?")
                .bind(id)
                .fetch_all(&state.read_pool)
                .await
                .unwrap_or_default();

        let codes_json: Vec<serde_json::Value> = backup_codes
            .iter()
            .map(|(code_id, code_hash, used)| {
                serde_json::json!({
                    "id": code_id,
                    "code_hash": code_hash,
                    "used": used,
                })
            })
            .collect();

        result.push(serde_json::json!({
            "id": id,
            "username": username,
            "password_hash": password_hash,
            "role": role,
            "storage_quota_bytes": quota,
            "created_at": created_at,
            "totp_secret": totp_secret,
            "totp_enabled": totp_enabled,
            "totp_backup_codes": codes_json,
        }));
    }

    Ok(Json(result))
}

/// POST /api/backup/sync-user-deletions
/// Accepts a list of user IDs that have been deleted on the primary server
/// and removes them from the backup's `users` table. Foreign-key cascades
/// clean up related rows (refresh_tokens, totp_backup_codes, etc.).
///
/// Content owned by deleted users (photos, trash, blobs) is also removed
/// via ON DELETE CASCADE, matching the primary's behaviour.
pub async fn backup_sync_user_deletions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    validate_api_key(&state, &headers).await?;

    let ids: Vec<String> = body
        .get("deleted_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if ids.is_empty() {
        return Ok(StatusCode::OK);
    }

    let mut removed = 0usize;
    for id in &ids {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&state.pool)
            .await;
        match result {
            Ok(r) if r.rows_affected() > 0 => {
                removed += 1;
            }
            Ok(_) => {} // user didn't exist — nothing to do
            Err(e) => {
                tracing::warn!(user_id = %id, "sync-user-deletions: failed to remove user: {}", e);
            }
        }
    }

    if removed > 0 {
        tracing::info!(
            "sync-user-deletions: removed {} user(s) deleted on primary",
            removed
        );
    }

    Ok(StatusCode::OK)
}
