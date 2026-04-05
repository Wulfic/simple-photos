//! Recovery engine — executes the actual disaster-recovery work.
//!
//! Phase 1 restores user accounts (with credentials and 2FA).
//! Phase 2 downloads missing photos, deduplicating by ID and file_path.

use chrono::Utc;

use crate::sanitize;
use super::models::*;

/// Execute the actual recovery from a backup server.
///
/// **Phase 1 — Users:** Pulls all user accounts (with credentials and 2FA)
/// from the backup via `GET /api/backup/list-users-full` and upserts them
/// locally. This ensures per-user photo ownership is preserved and users
/// can log in on the restored server immediately.
///
/// **Phase 2 — Photos:** Downloads all photos not already present locally
/// (by ID or file_path), preserving the original `user_id`. Falls back to
/// the admin's ID only when the referenced user doesn't exist locally.
pub(crate) async fn run_recovery(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
    server: &BackupServer,
    api_key: &Option<String>,
    admin_user_id: &str,
    recovery_id: &str,
) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .danger_accept_invalid_certs(true)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            update_recovery_log(pool, recovery_id, "error", 0, 0, Some(&e.to_string())).await;
            return;
        }
    };

    let base_url = format!("http://{}/api", server.address);

    // ── Phase 1: Restore user accounts ──────────────────────────────────
    let users_restored = recover_users(pool, &client, &base_url, api_key, &server.name).await;
    tracing::info!(
        "Recovery from '{}': restored {} user account(s)",
        server.name,
        users_restored
    );

    // Build a set of locally-known user IDs so we can validate ownership
    // when inserting photos. If the original owner doesn't exist locally
    // (e.g. partial recovery), we fall back to the admin.
    let local_user_ids: std::collections::HashSet<String> =
        match sqlx::query_scalar::<_, String>("SELECT id FROM users")
            .fetch_all(pool)
            .await
        {
            Ok(ids) => ids.into_iter().collect(),
            Err(e) => {
                tracing::warn!("Recovery: failed to query local users: {}", e);
                std::collections::HashSet::new()
            }
        };

    // ── Phase 2: Restore photos ─────────────────────────────────────────

    // Fetch remote photo list
    let mut list_req = client.get(format!("{}/backup/list", base_url));
    if let Some(ref key) = api_key {
        list_req = list_req.header("X-API-Key", key.as_str());
    }

    let remote_photos: Vec<BackupPhotoRecord> = match list_req.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json().await {
            Ok(photos) => photos,
            Err(e) => {
                update_recovery_log(
                    pool,
                    recovery_id,
                    "error",
                    0,
                    0,
                    Some(&format!("Failed to parse backup photo list: {}", e)),
                )
                .await;
                return;
            }
        },
        Ok(resp) => {
            update_recovery_log(
                pool,
                recovery_id,
                "error",
                0,
                0,
                Some(&format!("Backup server returned HTTP {}", resp.status())),
            )
            .await;
            return;
        }
        Err(e) => {
            update_recovery_log(
                pool,
                recovery_id,
                "error",
                0,
                0,
                Some(&format!("Failed to connect to backup server: {}", e)),
            )
            .await;
            return;
        }
    };

    tracing::info!(
        "Recovery from '{}': found {} photos on backup",
        server.name,
        remote_photos.len()
    );

    // Get local photo IDs for deduplication.
    // ID dedup handles re-recovery of the same backup.
    // file_path dedup is intentionally NOT done here because photo copies
    // ("Save Copy") share a file_path with the original.  The INSERT
    // WHERE NOT EXISTS handles true autoscan duplicates instead.
    let local_id_set: std::collections::HashSet<String> =
        match sqlx::query_scalar::<_, String>("SELECT id FROM photos")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows.into_iter().collect(),
            Err(e) => {
                update_recovery_log(pool, recovery_id, "error", 0, 0, Some(&e.to_string())).await;
                return;
            }
        };

    // Filter to photos not already on this server (by ID only).

    // Filter to photos not already on this server (by ID only).
    // We no longer filter by file_path because photo copies created by
    // "Save Copy" intentionally share a file_path with the original.
    // Filtering by file_path would prevent the second row from being
    // recovered.  The INSERT below handles true autoscan duplicates.
    let missing: Vec<&BackupPhotoRecord> = remote_photos
        .iter()
        .filter(|p| !local_id_set.contains(&p.id
        .collect();

    tracing::info!(
        "Recovery from '{}': {} photos to download ({} already exist locally by ID)",
        server.name,
        missing.len(),
        remote_photos.len() - missing.len()
    );

    let mut photos_recovered = 0i64;
    let mut bytes_recovered = 0i64;

    // Download and register each missing photo
    for photo in &missing {
        let mut dl_req = client.get(format!("{}/backup/download/{}", base_url, photo.id));
        if let Some(ref key) = api_key {
            dl_req = dl_req.header("X-API-Key", key.as_str());
        }

        let file_data = match dl_req.send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!(
                        "Recovery: failed to read bytes for {}: {}",
                        photo.filename,
                        e
                    );
                    continue;
                }
            },
            Ok(resp) => {
                tracing::warn!(
                    "Recovery: backup returned HTTP {} for photo {}",
                    resp.status(),
                    photo.filename
                );
                continue;
            }
            Err(e) => {
                tracing::warn!("Recovery: download failed for {}: {}", photo.filename, e);
                continue;
            }
        };

        // Determine storage path — use the original file_path from the backup.
        // SECURITY: validate the remote-supplied path to prevent directory traversal.
        let file_path = &photo.file_path;
        if let Err(reason) = sanitize::validate_relative_path(file_path) {
            tracing::warn!(
                "Recovery: skipping {} — unsafe file_path '{}': {}",
                photo.filename,
                file_path,
                reason
            );
            continue;
        }
        let full_path = storage_root.join(file_path);

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                tracing::warn!("Recovery: failed to create dir for {}: {}", file_path, e);
                continue;
            }
        }

        // Write file to disk
        if let Err(e) = tokio::fs::write(&full_path, &file_data).await {
            tracing::warn!("Recovery: failed to write {}: {}", file_path, e);
            continue;
        }

        // Preserve original user_id if the user exists locally; fall back to admin.
        let effective_user_id = if local_user_ids.contains(&photo.user_id) {
            &photo.user_id
        } else {
            tracing::debug!(
                "Recovery: user_id '{}' not found locally for photo {}; assigning to admin",
                photo.user_id,
                photo.id
            );
            admin_user_id
        };

        // Register in the photos table — preserve the original photo ID from
        // the backup so delta-sync and re-recovery can deduplicate correctly.
        let now = Utc::now().to_rfc3339();
        let thumb_filename = format!("{}.thumb.jpg", photo.id);
        let thumb_rel = format!(".thumbnails/{}", thumb_filename);

        // INSERT OR IGNORE handles id conflicts (re-recovery idempotency).
        // The WHERE NOT EXISTS guard prevents creating a duplicate when the
        // same file_path was registered by a local autoscan (different ID,
        // same file).  We only check rows that have a photo_hash (canonical
        // INSERT OR IGNORE handles id conflicts (re-recovery idempotency).
        // The WHERE NOT EXISTS guard prevents creating a duplicate when the
        // same file_path was registered by a local autoscan (different ID,
        // same file).  We only check rows that have a photo_hash (canonical
        // rows) — copy rows (photo_hash IS NULL) intentionally share a
        // file_path with the original and must be allowed through.
        let result = sqlx::query(
            "INSERT OR IGNORE INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
             thumb_path, created_at, is_favorite, camera_model, photo_hash, crop_metadata) \
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19 \
             WHERE NOT EXISTS (SELECT 1 FROM photos WHERE file_path = ?4 AND photo_hash IS NOT NULL AND id != ?1sh, crop_metadata) \
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19 \
             WHERE NOT EXISTS (SELECT 1 FROM photos WHERE file_path = ?4 AND photo_hash IS NOT NULL AND id != ?1)",
        )
        .bind(&photo.id)           // ?1
        .bind(effective_user_id)    // ?2
        .bind(&photo.filename)     // ?3
        .bind(file_path)           // ?4
        .bind(&photo.mime_type)    // ?5
        .bind(&photo.media_type)   // ?6
        .bind(file_data.len() as i64) // ?7
        .bind(photo.width)         // ?8
        .bind(photo.height)        // ?9
        .bind(photo.duration_secs) // ?10
        .bind(&photo.taken_at)     // ?11
        .bind(photo.latitude)      // ?12
        .bind(photo.longitude)     // ?13
        .bind(&thumb_rel)          // ?14
        .bind(&now)                // ?15
        .bind(photo.is_favorite)   // ?16
        .bind(&photo.camera_model) // ?17
        .bind(&photo.photo_hash)   // ?18
        .bind(&photo.crop_metadata) // ?19
        .execute(pool)
        .await;

        match result {
            Err(e) => {
                tracing::warn!("Recovery: failed to register {}: {}", photo.filename, e);
                // Clean up the written file on DB failure
                let _ = tokio::fs::remove_file(&full_path).await;
                continue;
            }
            Ok(res) if res.rows_affected() == 0 => {
                // Row was skipped (ID conflict or file_path already exists).
                // Remove the file we just wrote — we don't need it.
                let _ = tokio::fs::remove_file(&full_path).await;
                continue;
            }
            Ok(_) => {}
        }

        photos_recovered += 1;
        bytes_recovered += file_data.len() as i64;
    }

    // Update recovery log
    update_recovery_log(
        pool,
        recovery_id,
        "success",
        photos_recovered,
        bytes_recovered,
        None,
    )
    .await;

    tracing::info!(
        "Recovery from '{}' complete: {} user(s), {} photos, {} bytes recovered",
        server.name,
        users_restored,
        photos_recovered,
        bytes_recovered
    );
}

// ── User Recovery Helper ─────────────────────────────────────────────────────

/// Pull all user accounts from a backup server and upsert them locally.
/// Returns the number of users successfully restored.
///
/// Uses `GET /api/backup/list-users-full` which returns full credentials
/// (password_hash, totp_secret, totp_enabled, totp_backup_codes).
/// Each user is upserted via INSERT ... ON CONFLICT(id) DO UPDATE so
/// re-running recovery is idempotent and won't clobber local changes
/// if the user already exists.
async fn recover_users(
    pool: &sqlx::SqlitePool,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    server_name: &str,
) -> usize {
    let mut req = client.get(format!("{}/backup/list-users-full", base_url));
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    let remote_users: Vec<serde_json::Value> = match req.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json().await {
            Ok(users) => users,
            Err(e) => {
                tracing::warn!(
                    "Recovery from '{}': failed to parse user list: {} — skipping user restore",
                    server_name,
                    e
                );
                return 0;
            }
        },
        Ok(resp) => {
            // Graceful degradation: if the backup doesn't support this endpoint
            // (older version), log a warning and continue to photo recovery.
            tracing::warn!(
                "Recovery from '{}': list-users-full returned HTTP {} — \
                 skipping user restore (backup may be running an older version)",
                server_name,
                resp.status()
            );
            return 0;
        }
        Err(e) => {
            tracing::warn!(
                "Recovery from '{}': failed to fetch users: {} — skipping user restore",
                server_name,
                e
            );
            return 0;
        }
    };

    tracing::info!(
        "Recovery from '{}': found {} user account(s) on backup",
        server_name,
        remote_users.len()
    );

    let mut restored = 0usize;
    for user in &remote_users {
        let id = match user.get("id").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => continue,
        };
        let username = match user.get("username").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => continue,
        };
        let password_hash = match user.get("password_hash").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => continue,
        };
        let role = user
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("user");
        let quota = user
            .get("storage_quota_bytes")
            .and_then(|v| v.as_i64())
            .unwrap_or(10_737_418_240);
        let created_at = user
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let totp_secret: Option<&str> = user.get("totp_secret").and_then(|v| v.as_str());
        let totp_enabled: i32 = user
            .get("totp_enabled")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        // Upsert user — ON CONFLICT(id) preserves existing local user if
        // already present, but updates credentials to match the backup.
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
        .bind(id)
        .bind(username)
        .bind(password_hash)
        .bind(created_at)
        .bind(quota)
        .bind(role)
        .bind(totp_secret)
        .bind(totp_enabled)
        .execute(pool)
        .await;

        if let Err(e) = result {
            // Username collision (different id, same username) — skip
            // rather than risk data loss during recovery.
            tracing::warn!(
                "Recovery: failed to restore user '{}' (id={}): {}",
                username,
                id,
                e
            );
            continue;
        }

        // Restore TOTP backup codes
        if let Some(codes) = user.get("totp_backup_codes").and_then(|v| v.as_array()) {
            // Clear existing codes for this user first (idempotent re-runs)
            let _ = sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
                .bind(id)
                .execute(pool)
                .await;

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

                let _ = sqlx::query(
                    "INSERT OR REPLACE INTO totp_backup_codes (id, user_id, code_hash, used) \
                     VALUES (?, ?, ?, ?)",
                )
                .bind(code_id)
                .bind(id)
                .bind(code_hash)
                .bind(used)
                .execute(pool)
                .await;
            }
        }

        restored += 1;
    }

    restored
}

/// Update the sync log entry used for tracking recovery progress.
pub(crate) async fn update_recovery_log(
    pool: &sqlx::SqlitePool,
    log_id: &str,
    status: &str,
    photos_synced: i64,
    bytes_synced: i64,
    error: Option<&str>,
) {
    let now = Utc::now().to_rfc3339();
    if let Err(e) = sqlx::query(
        "UPDATE backup_sync_log SET completed_at = ?, status = ?, photos_synced = ?, \
         bytes_synced = ?, error = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(status)
    .bind(photos_synced)
    .bind(bytes_synced)
    .bind(error)
    .bind(log_id)
    .execute(pool)
    .await
    {
        tracing::error!("Failed to update recovery log {}: {}", log_id, e);
    }
}
