//! User account synchronization from primary → backup server.
//!
//! Handles two concerns:
//! - **Upserting users**: sends full credentials (password hash, TOTP secret,
//!   backup codes) so users can authenticate on the backup.
//! - **Deleting users**: detects users removed from the primary and propagates
//!   those deletions to the backup.

use std::collections::HashSet;

/// Sync all user accounts from the primary to the backup server.
/// Sends full credentials (password hash, TOTP secret/enabled, TOTP backup
/// codes) so users can log in on the backup. All users are sent on every
/// run so that password changes, role changes, and 2FA changes propagate.
pub async fn sync_users_to_backup(
    pool: &sqlx::SqlitePool,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
) {
    // Fetch all user records including credentials
    let users: Vec<(
        String,         // id
        String,         // username
        String,         // password_hash
        String,         // role
        i64,            // storage_quota_bytes
        String,         // created_at
        Option<String>, // totp_secret
        i32,            // totp_enabled
    )> = match sqlx::query_as(
        "SELECT id, username, password_hash, role, storage_quota_bytes, \
         created_at, totp_secret, totp_enabled FROM users",
    )
    .fetch_all(pool)
    .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("Could not fetch users for backup sync: {}", e);
            return;
        }
    };

    let mut synced = 0u32;
    for (id, username, password_hash, role, quota, created_at, totp_secret, totp_enabled) in &users
    {
        // Fetch TOTP backup codes for this user
        let backup_codes: Vec<(String, String, i32)> =
            sqlx::query_as("SELECT id, code_hash, used FROM totp_backup_codes WHERE user_id = ?")
                .bind(id)
                .fetch_all(pool)
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

        let body = serde_json::json!({
            "id": id,
            "username": username,
            "password_hash": password_hash,
            "role": role,
            "storage_quota_bytes": quota,
            "created_at": created_at,
            "totp_secret": totp_secret,
            "totp_enabled": totp_enabled,
            "totp_backup_codes": codes_json,
        });
        let mut req = client
            .post(format!("{}/backup/upsert-user", base_url))
            .json(&body);
        if let Some(ref key) = api_key {
            req = req.header("X-API-Key", key.as_str());
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => synced += 1,
            Ok(resp) => tracing::warn!(
                "Failed to upsert user {} on backup: HTTP {}",
                id,
                resp.status()
            ),
            Err(e) => tracing::warn!("Failed to upsert user {} on backup: {}", id, e),
        }
    }

    if synced > 0 {
        tracing::info!("Synced {} user account(s) to backup server", synced);
    }
}

/// Detect users deleted on the primary and propagate those deletions to
/// the backup server. Compares the primary's user IDs against the remote's
/// `GET /api/backup/list-users` response. Any user that exists on the
/// remote but not locally has been deleted and should be removed.
pub async fn sync_user_deletions_to_backup(
    pool: &sqlx::SqlitePool,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    server_name: &str,
) {
    // Fetch remote user IDs
    let mut req = client.get(format!("{}/backup/list-users", base_url));
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    #[derive(serde::Deserialize)]
    struct UserIdOnly {
        id: String,
    }

    let remote_users: Vec<UserIdOnly> = match req.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json().await {
            Ok(users) => users,
            Err(e) => {
                tracing::warn!(
                    "sync-user-deletions to '{}': failed to parse remote user list: {}",
                    server_name,
                    e
                );
                return;
            }
        },
        Ok(resp) => {
            tracing::warn!(
                "sync-user-deletions to '{}': list-users returned HTTP {}",
                server_name,
                resp.status()
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                "sync-user-deletions to '{}': failed to fetch remote users: {}",
                server_name,
                e
            );
            return;
        }
    };

    if remote_users.is_empty() {
        return;
    }

    // Fetch local user IDs
    let local_ids: HashSet<String> = match sqlx::query_scalar::<_, String>("SELECT id FROM users")
        .fetch_all(pool)
        .await
    {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            tracing::warn!("sync-user-deletions: failed to query local users: {}", e);
            return;
        }
    };

    // Users on the remote that no longer exist locally have been deleted
    let to_delete: Vec<&String> = remote_users
        .iter()
        .map(|u| &u.id)
        .filter(|id| !local_ids.contains(*id))
        .collect();

    if to_delete.is_empty() {
        return;
    }

    let payload = serde_json::json!({ "deleted_ids": to_delete });
    let url = format!("{}/backup/sync-user-deletions", base_url);
    let mut req = client.post(&url).json(&payload);
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                server = %server_name,
                count = to_delete.len(),
                "Synced user deletions to backup"
            );
        }
        Ok(resp) => {
            tracing::warn!(
                server = %server_name,
                status = %resp.status(),
                "sync-user-deletions returned non-success status"
            );
        }
        Err(e) => {
            tracing::warn!(
                server = %server_name,
                "sync-user-deletions request failed: {}",
                e
            );
        }
    }
}
