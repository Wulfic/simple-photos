//! Backup sync engine — pushes local photos + trash to a remote backup
//! server via `POST /api/backup/receive`.
//!
//! **Features:**
//! - **Delta sync** — queries the remote's existing photo/trash IDs before
//!   transferring, so only new or changed items cross the network.
//! - **Concurrency lock** — a per-server guard prevents overlapping syncs
//!   (manual `trigger_sync` vs. background `background_sync_task`).
//! - **Checksum verification** — each transfer includes an `X-Content-Hash`
//!   (SHA-256) header so the receiver can verify data integrity.
//! - **Per-file error tracking** — individual failures are counted and the
//!   sync log records `"partial"` status with error details when some
//!   transfers fail.
//!
//! **Remaining limitation:**
//! - **Plain HTTP** — the API key is sent in cleartext unless a TLS-terminating
//!   proxy sits between the two servers.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use percent_encoding::{utf8_percent_encode, CONTROLS};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::models::*;

// ── Full-metadata structs for sync ───────────────────────────────────────────

/// All metadata columns needed to faithfully replicate a photo entry.
#[derive(Debug, sqlx::FromRow)]
struct PhotoToSync {
    id: String,
    user_id: String,
    filename: String,
    file_path: String,
    mime_type: String,
    media_type: String,
    size_bytes: i64,
    taken_at: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    width: i64,
    height: i64,
    duration_secs: Option<f64>,
    camera_model: Option<String>,
    is_favorite: bool,
    photo_hash: Option<String>,
    crop_metadata: Option<String>,
    created_at: String,
}

/// All metadata columns needed to faithfully replicate a trash item.
#[derive(Debug, sqlx::FromRow)]
struct TrashToSync {
    id: String,
    /// The original photo UUID — different from `id` (the trash row UUID).
    /// Used by the backup to remove the item from its own `photos` table.
    photo_id: String,
    user_id: String,
    filename: String,
    file_path: String,
    mime_type: String,
    media_type: String,
    size_bytes: i64,
    taken_at: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    width: i64,
    height: i64,
    duration_secs: Option<f64>,
    camera_model: Option<String>,
    is_favorite: bool,
    photo_hash: Option<String>,
    crop_metadata: Option<String>,
    deleted_at: String,
    expires_at: String,
}

// ── Concurrency Lock ─────────────────────────────────────────────────────────

/// Tracks which backup server IDs have an active sync in progress.
/// Prevents overlapping syncs to the same server (manual trigger vs. background).
fn active_syncs() -> &'static std::sync::Mutex<HashSet<String>> {
    static INSTANCE: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> =
        std::sync::OnceLock::new();
    INSTANCE.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

/// RAII guard that removes the server ID from the active set on drop.
pub struct SyncGuard {
    server_id: String,
}

impl Drop for SyncGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = active_syncs().lock() {
            set.remove(&self.server_id);
        }
    }
}

/// Try to acquire the sync lock for a server. Returns `Some(SyncGuard)` if
/// no other sync is running for that server, or `None` if one is already active.
pub fn try_acquire_sync(server_id: &str) -> Option<SyncGuard> {
    let mut set = active_syncs().lock().ok()?;
    if set.contains(server_id) {
        None
    } else {
        set.insert(server_id.to_string());
        Some(SyncGuard {
            server_id: server_id.to_string(),
        })
    }
}

/// POST /api/admin/backup/servers/:id/sync
/// Trigger an immediate sync to a backup server (admin only).
pub async fn trigger_sync(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    let server = sqlx::query_as::<_, BackupServer>(
        "SELECT id, name, address, sync_frequency_hours, last_sync_at, \
         last_sync_status, last_sync_error, enabled, created_at \
         FROM backup_servers WHERE id = ?",
    )
    .bind(&server_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    if !server.enabled {
        return Err(AppError::BadRequest("Backup server is disabled".into()));
    }

    // Prevent overlapping syncs to the same server
    let guard = try_acquire_sync(&server_id).ok_or_else(|| {
        AppError::BadRequest("A sync is already in progress for this server".into())
    })?;

    // Create sync log entry
    let log_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO backup_sync_log (id, server_id, started_at, status) VALUES (?, ?, ?, 'running')",
    )
    .bind(&log_id)
    .bind(&server_id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    // Spawn the sync as a background task
    let pool = state.pool.clone();
    let storage_root = (**state.storage_root.load()).clone();
    let api_key: Option<String> =
        sqlx::query_scalar("SELECT api_key FROM backup_servers WHERE id = ?")
            .bind(&server_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();

    let log_id_clone = log_id.clone();
    tokio::spawn(async move {
        // `guard` is moved into this future — the concurrency lock is held
        // for the entire duration of run_sync and released on drop.
        let _guard = guard;
        run_sync(&pool, &storage_root, &server, &api_key, &log_id_clone).await;
    });

    Ok(Json(serde_json::json!({
        "message": "Sync started",
        "sync_id": log_id,
    })))
}

// ── Sync Engine ──────────────────────────────────────────────────────────────

/// Run the actual sync operation against a backup server.
///
/// **Delta sync:** queries the remote's existing photo & trash IDs first, then
/// only transfers items the remote doesn't already have.
///
/// **Checksums:** each file is sent with an `X-Content-Hash` (hex-encoded
/// SHA-256) header so the receiver can verify data integrity.
///
/// **Per-file tracking:** individual successes and failures are counted. The
/// sync log records `"partial"` when some transfers fail, or `"success"` when
/// all succeed.
async fn run_sync(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
    server: &BackupServer,
    api_key: &Option<String>,
    log_id: &str,
) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        // Accept self-signed certs — backup servers on a LAN often use
        // untrusted TLS or sit behind a self-signed reverse proxy.
        .danger_accept_invalid_certs(true)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            update_sync_log(pool, log_id, "error", 0, 0, Some(&e.to_string())).await;
            return;
        }
    };

    let base_url = format!("http://{}/api", server.address);
    let has_key = api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false);
    tracing::info!(
        server_name = %server.name,
        server_address = %server.address,
        base_url = %base_url,
        has_api_key = has_key,
        log_id = %log_id,
        "Starting sync to backup server"
    );

    let mut photos_synced = 0i64;
    let mut bytes_synced = 0i64;
    let mut failures = 0i64;
    let mut last_error: Option<String> = None;

    // ── Pre-flight: verify the backup server is reachable ───────────────
    // A quick health check avoids wasting time on N individual file failures
    // when the address is simply wrong or the server is down.
    let health_url = format!("http://{}/health", server.address);
    match client.get(&health_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                server = %server.name,
                address = %server.address,
                "Backup server health check passed"
            );
        }
        Ok(resp) => {
            let msg = format!(
                "Backup server health check returned HTTP {} at {}",
                resp.status(),
                health_url
            );
            tracing::error!("{}", msg);
            update_sync_log(pool, log_id, "error", 0, 0, Some(&msg)).await;
            let now = Utc::now().to_rfc3339();
            if let Err(e) = sqlx::query(
                "UPDATE backup_servers SET last_sync_at = ?, last_sync_status = 'error', \
                 last_sync_error = ? WHERE id = ?",
            )
            .bind(&now)
            .bind(&msg)
            .bind(&server.id)
            .execute(pool)
            .await
            {
                tracing::warn!(
                    "Failed to update sync error status for server {}: {}",
                    server.id,
                    e
                );
            }
            return;
        }
        Err(e) => {
            let msg = format!(
                "Cannot reach backup server at {} — {}\n\
                 Hint: verify the registered address in backup_servers matches \
                 the backup's externally-reachable host:port (check base_url in \
                 the backup's config.toml).",
                health_url, e
            );
            tracing::error!("{}", msg);
            update_sync_log(pool, log_id, "error", 0, 0, Some(&msg)).await;
            let now = Utc::now().to_rfc3339();
            if let Err(e) = sqlx::query(
                "UPDATE backup_servers SET last_sync_at = ?, last_sync_status = 'error', \
                 last_sync_error = ? WHERE id = ?",
            )
            .bind(&now)
            .bind(&msg)
            .bind(&server.id)
            .execute(pool)
            .await
            {
                tracing::warn!(
                    "Failed to update sync error status for server {}: {}",
                    server.id,
                    e
                );
            }
            return;
        }
    }

    // ── Delta: fetch IDs the remote already has ──────────────────────────
    let remote_photo_ids: HashSet<String> =
        fetch_remote_ids(&client, &base_url, "/backup/list", api_key).await;
    let remote_trash_ids: HashSet<String> =
        fetch_remote_ids(&client, &base_url, "/backup/list-trash", api_key).await;

    // ── Phase 0a: purge photos that have since been deleted on the primary ──
    // Compute the IDs that the remote still has in its gallery BUT are now in
    // the primary's trash.  These are items deleted after the last sync; they
    // must be removed from the backup gallery even when the delta logic would
    // otherwise skip the file transfer (because the trash entry is already
    // present on the remote).
    //
    // IMPORTANT: use `photo_id` (the original photo UUID), NOT `id` (the
    // trash row UUID) — the backup's photos table stores original photo UUIDs.
    {
        let primary_trash_ids: Vec<String> = match sqlx::query_scalar::<_, String>(
            "SELECT photo_id FROM trash_items",
        )
        .fetch_all(pool)
        .await
        {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!("Failed to fetch primary trash photo_ids for deletion sync: {}", e);
                vec![]
            }
        };

        // Only send IDs that the remote's gallery actually contains — keeps the
        // payload minimal and avoids no-op deletes on the remote.
        let to_delete: Vec<&String> = primary_trash_ids
            .iter()
            .filter(|id| remote_photo_ids.contains(*id))
            .collect();

        if !to_delete.is_empty() {
            let payload = serde_json::json!({ "deleted_ids": to_delete });
            let url = format!("{}/backup/sync-deletions", base_url);
            let mut req = client.post(&url).json(&payload);
            if let Some(ref key) = api_key {
                req = req.header("X-API-Key", key.as_str());
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::info!(
                        server = %server.name,
                        count = to_delete.len(),
                        "Purged deleted photos from backup gallery"
                    );
                }
                Ok(resp) => {
                    tracing::warn!(
                        server = %server.name,
                        status = %resp.status(),
                        "sync-deletions returned non-success status"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        server = %server.name,
                        "sync-deletions request failed: {}",
                        e
                    );
                }
            }
        }
    }

    // ── Phase 0: sync user accounts ──────────────────────────────────────
    // Must run before photos/trash so user_id foreign keys resolve correctly.
    sync_users_to_backup(pool, &client, &base_url, api_key).await;

    // ── Phase 0b: sync user deletions ────────────────────────────────────
    // Users deleted on the primary must also be removed from the backup.
    // Compare the primary's user IDs against the remote's and send deletions.
    sync_user_deletions_to_backup(pool, &client, &base_url, api_key, &server.name).await;

    // Pre-fetch ALL photo tags in one query to avoid N+1 inside the transfer loop.
    let all_tags: std::collections::HashMap<String, Vec<String>> = {
        match sqlx::query_as::<_, (String, String)>(
            "SELECT photo_id, tag FROM photo_tags ORDER BY photo_id",
        )
        .fetch_all(pool)
        .await
        {
            Ok(rows) => {
                let mut map: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();
                for (photo_id, tag) in rows {
                    map.entry(photo_id).or_default().push(tag);
                }
                map
            }
            Err(e) => {
                tracing::warn!("Failed to fetch photo tags for sync: {}", e);
                std::collections::HashMap::new()
            }
        }
    };

    // Check whether audio files should be included in sync
    let audio_backup_enabled: bool = sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(false);

    // 1. Sync photos — only those the remote doesn't have yet
    let photos: Vec<PhotoToSync> = {
        let query = if audio_backup_enabled {
            "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, taken_at, latitude, longitude, \
             width, height, duration_secs, camera_model, is_favorite, photo_hash, \
             crop_metadata, created_at FROM photos"
        } else {
            "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, taken_at, latitude, longitude, \
             width, height, duration_secs, camera_model, is_favorite, photo_hash, \
             crop_metadata, created_at FROM photos WHERE media_type != 'audio'"
        };
        match sqlx::query_as::<_, PhotoToSync>(query)
            .fetch_all(pool)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                update_sync_log(pool, log_id, "error", 0, 0, Some(&e.to_string())).await;
                return;
            }
        }
    };

    let photos_to_sync: Vec<_> = photos
        .iter()
        .filter(|p| !remote_photo_ids.contains(&p.id))
        .collect();

    tracing::info!(
        "Backup sync to '{}': {}/{} photos need transfer (delta)",
        server.name,
        photos_to_sync.len(),
        photos.len()
    );

    for photo in &photos_to_sync {
        let tags = all_tags.get(&photo.id).cloned().unwrap_or_default();
        let mut extra: Vec<(String, String)> = vec![
            ("X-User-Id".to_string(), photo.user_id.clone()),
            (
                "X-Original-Created-At".to_string(),
                photo.created_at.clone(),
            ),
            (
                "X-Filename".to_string(),
                utf8_percent_encode(&photo.filename, CONTROLS).to_string(),
            ),
            ("X-Mime-Type".to_string(), photo.mime_type.clone()),
            ("X-Media-Type".to_string(), photo.media_type.clone()),
            ("X-Width".to_string(), photo.width.to_string()),
            ("X-Height".to_string(), photo.height.to_string()),
            (
                "X-Is-Favorite".to_string(),
                if photo.is_favorite { "1" } else { "0" }.to_string(),
            ),
        ];
        if let Some(ref v) = photo.taken_at {
            extra.push(("X-Taken-At".to_string(), v.clone()));
        }
        if let Some(v) = photo.latitude {
            extra.push(("X-Latitude".to_string(), v.to_string()));
        }
        if let Some(v) = photo.longitude {
            extra.push(("X-Longitude".to_string(), v.to_string()));
        }
        if let Some(v) = photo.duration_secs {
            extra.push(("X-Duration-Secs".to_string(), v.to_string()));
        }
        if let Some(ref v) = photo.camera_model {
            extra.push((
                "X-Camera-Model".to_string(),
                utf8_percent_encode(v, CONTROLS).to_string(),
            ));
        }
        if let Some(ref v) = photo.photo_hash {
            extra.push(("X-Photo-Hash".to_string(), v.clone()));
        }
        if let Some(ref v) = photo.crop_metadata {
            extra.push((
                "X-Crop-Metadata".to_string(),
                utf8_percent_encode(v, CONTROLS).to_string(),
            ));
        }
        if !tags.is_empty() {
            let tags_str = tags
                .iter()
                .map(|t| utf8_percent_encode(t, CONTROLS).to_string())
                .collect::<Vec<_>>()
                .join(",");
            extra.push(("X-Tags".to_string(), tags_str));
        }
        match send_file(
            &client,
            &base_url,
            api_key,
            storage_root,
            &photo.id,
            &photo.file_path,
            "photos",
            &extra,
        )
        .await
        {
            Ok(()) => {
                photos_synced += 1;
                bytes_synced += photo.size_bytes;
            }
            Err(e) => {
                failures += 1;
                last_error = Some(e.clone());
                tracing::warn!("Backup sync failed for photo {}: {}", photo.id, e);
            }
        }
    }

    // 2. Sync trash items — only those the remote doesn't have yet
    let trash_items: Vec<TrashToSync> = match sqlx::query_as::<_, TrashToSync>(
        "SELECT id, photo_id, user_id, filename, file_path, mime_type, media_type, size_bytes, taken_at, latitude, longitude, \
         width, height, duration_secs, camera_model, is_favorite, photo_hash, \
         crop_metadata, deleted_at, expires_at FROM trash_items",
    )
    .fetch_all(pool)
    .await
    {
        Ok(t) => t,
        Err(e) => {
            update_sync_log(
                pool,
                log_id,
                "error",
                photos_synced,
                bytes_synced,
                Some(&e.to_string()),
            )
            .await;
            return;
        }
    };

    let trash_to_sync: Vec<_> = trash_items
        .iter()
        .filter(|t| !remote_trash_ids.contains(&t.id))
        .collect();

    tracing::info!(
        "Backup sync to '{}': {}/{} trash items need transfer (delta)",
        server.name,
        trash_to_sync.len(),
        trash_items.len()
    );

    for item in &trash_to_sync {
        let mut extra: Vec<(String, String)> = vec![
            ("X-User-Id".to_string(), item.user_id.clone()),
            ("X-Original-Created-At".to_string(), item.deleted_at.clone()),
            ("X-Deleted-At".to_string(), item.deleted_at.clone()),
            ("X-Expires-At".to_string(), item.expires_at.clone()),
            // Send the original photo UUID so the backup can evict the item
            // from its gallery (photos.id = photo_id, not the trash row id).
            ("X-Original-Photo-Id".to_string(), item.photo_id.clone()),
            (
                "X-Filename".to_string(),
                utf8_percent_encode(&item.filename, CONTROLS).to_string(),
            ),
            ("X-Mime-Type".to_string(), item.mime_type.clone()),
            ("X-Media-Type".to_string(), item.media_type.clone()),
            ("X-Width".to_string(), item.width.to_string()),
            ("X-Height".to_string(), item.height.to_string()),
            (
                "X-Is-Favorite".to_string(),
                if item.is_favorite { "1" } else { "0" }.to_string(),
            ),
        ];
        if let Some(ref v) = item.taken_at {
            extra.push(("X-Taken-At".to_string(), v.clone()));
        }
        if let Some(v) = item.latitude {
            extra.push(("X-Latitude".to_string(), v.to_string()));
        }
        if let Some(v) = item.longitude {
            extra.push(("X-Longitude".to_string(), v.to_string()));
        }
        if let Some(v) = item.duration_secs {
            extra.push(("X-Duration-Secs".to_string(), v.to_string()));
        }
        if let Some(ref v) = item.camera_model {
            extra.push((
                "X-Camera-Model".to_string(),
                utf8_percent_encode(v, CONTROLS).to_string(),
            ));
        }
        if let Some(ref v) = item.photo_hash {
            extra.push(("X-Photo-Hash".to_string(), v.clone()));
        }
        if let Some(ref v) = item.crop_metadata {
            extra.push((
                "X-Crop-Metadata".to_string(),
                utf8_percent_encode(v, CONTROLS).to_string(),
            ));
        }
        match send_file(
            &client,
            &base_url,
            api_key,
            storage_root,
            &item.id,
            &item.file_path,
            "trash",
            &extra,
        )
        .await
        {
            Ok(()) => {
                photos_synced += 1;
                bytes_synced += item.size_bytes;
            }
            Err(e) => {
                failures += 1;
                last_error = Some(e.clone());
                tracing::warn!("Backup sync failed for trash {}: {}", item.id, e);
            }
        }
    }

    // 3. Determine final status based on failure count
    let (status, error_detail) = if failures == 0 {
        ("success", None)
    } else if photos_synced == 0 && failures > 0 {
        (
            "error",
            Some(format!(
                "All {} transfers failed. Last error: {}",
                failures,
                last_error.as_deref().unwrap_or("unknown")
            )),
        )
    } else {
        (
            "partial",
            Some(format!(
                "{} of {} transfers failed. Last error: {}",
                failures,
                photos_synced + failures,
                last_error.as_deref().unwrap_or("unknown")
            )),
        )
    };

    update_sync_log(
        pool,
        log_id,
        status,
        photos_synced,
        bytes_synced,
        error_detail.as_deref(),
    )
    .await;

    let now = Utc::now().to_rfc3339();
    // Mirror the fine-grained status into backup_servers so the background
    // scheduler can distinguish a total failure ("error") from a partial one.
    let db_status = status;
    let db_error = error_detail.as_deref();

    if let Err(e) = sqlx::query(
        "UPDATE backup_servers SET last_sync_at = ?, last_sync_status = ?, \
         last_sync_error = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(db_status)
    .bind(db_error)
    .bind(&server.id)
    .execute(pool)
    .await
    {
        tracing::warn!(server_id = %server.id, error = %e, "Failed to update backup server sync status");
    }

    tracing::info!(
        "Backup sync to '{}' complete: {} synced, {} failed, {} bytes",
        server.name,
        photos_synced,
        failures,
        bytes_synced
    );
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Fetch the set of IDs the remote backup already has for a given list endpoint.
/// Returns an empty set on any error (graceful degradation to full sync).
async fn fetch_remote_ids(
    client: &reqwest::Client,
    base_url: &str,
    path: &str,
    api_key: &Option<String>,
) -> HashSet<String> {
    let mut req = client.get(format!("{}{}", base_url, path));
    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            // Parse as array of objects with an "id" field
            #[derive(serde::Deserialize)]
            struct IdOnly {
                id: String,
            }
            match resp.json::<Vec<IdOnly>>().await {
                Ok(items) => {
                    tracing::info!(
                        path = %path,
                        count = items.len(),
                        "Fetched remote ID list successfully"
                    );
                    items.into_iter().map(|i| i.id).collect()
                }
                Err(e) => {
                    tracing::warn!("Failed to parse remote ID list from {}: {}", path, e);
                    HashSet::new()
                }
            }
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::error!(
                path = %path,
                status = %status,
                body = %body,
                "Remote ID list fetch failed — this likely means the backup is unreachable or auth failed"
            );
            // Return an error sentinel so the caller can abort early
            // instead of trying to push every file only to get N failures.
            tracing::warn!(
                "Remote {} returned HTTP {} — falling back to full sync",
                path,
                status
            );
            HashSet::new()
        }
        Err(e) => {
            tracing::warn!(
                "Failed to fetch remote IDs from {}: {} — falling back to full sync",
                path,
                e
            );
            HashSet::new()
        }
    }
}

/// Read a file from disk, compute its SHA-256 checksum, and POST it to the
/// backup server with integrity headers. Returns `Ok(())` on success or
/// `Err(description)` on failure.
async fn send_file(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    storage_root: &std::path::Path,
    item_id: &str,
    file_path: &str,
    source: &str,
    extra_headers: &[(String, String)],
) -> Result<(), String> {
    let full_path = storage_root.join(file_path);
    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err("file not found on disk".to_string());
    }

    let file_data = tokio::fs::read(&full_path)
        .await
        .map_err(|e| format!("read error: {}", e))?;

    // Compute SHA-256 checksum for integrity verification
    let hash = Sha256::digest(&file_data);
    let hash_hex = hex::encode(hash);

    // Percent-encode non-ASCII characters in file_path so the header
    // value stays within visible-ASCII (RFC 7230 §3.2.6).  The receiver
    // will percent-decode before using the path.
    let encoded_path = utf8_percent_encode(file_path, CONTROLS).to_string();

    let mut req = client
        .post(format!("{}/backup/receive", base_url))
        .header("X-Photo-Id", item_id)
        .header("X-File-Path", encoded_path.as_str())
        .header("X-Source", source)
        .header("X-Content-Hash", hash_hex.as_str())
        .body(file_data);

    // Attach all metadata headers so the backup stores a faithful replica
    for (name, value) in extra_headers {
        if let Ok(hv) = reqwest::header::HeaderValue::from_str(value) {
            req = req.header(name.as_str(), hv);
        }
    }

    if let Some(ref key) = api_key {
        req = req.header("X-API-Key", key.as_str());
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => Ok(()),
        Ok(resp) => Err(format!("HTTP {}", resp.status())),
        Err(e) => Err(e.to_string()),
    }
}

// ── User Sync Helpers ────────────────────────────────────────────────────────

/// Sync all user accounts from the primary to the backup server.
/// Sends full credentials (password hash, TOTP secret/enabled, TOTP backup
/// codes) so users can log in on the backup. All users are sent on every
/// run so that password changes, role changes, and 2FA changes propagate.
async fn sync_users_to_backup(
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
async fn sync_user_deletions_to_backup(
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

async fn update_sync_log(
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
        // Without this update the sync log row stays stuck in "running" state
        // permanently, so operators should be alerted.
        tracing::error!(log_id = log_id, error = %e, "Failed to update backup sync log — row stuck in 'running' state");
    }
}

/// Background task: periodically sync to all enabled backup servers
/// based on their configured frequency.
pub async fn background_sync_task(pool: sqlx::SqlitePool, storage_root: std::path::PathBuf) {
    // Check every 5 minutes so newly-paired servers get synced quickly.
    // The per-server `sync_frequency_hours` still controls how often each
    // individual server is actually synced — this interval only affects
    // how often we *check* whether any server is due.
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));

    loop {
        interval.tick().await;

        let servers = match sqlx::query_as::<_, BackupServer>(
            "SELECT id, name, address, sync_frequency_hours, last_sync_at, \
             last_sync_status, last_sync_error, enabled, created_at \
             FROM backup_servers WHERE enabled = 1",
        )
        .fetch_all(&pool)
        .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to query backup servers: {}", e);
                continue;
            }
        };

        for server in &servers {
            // Check if it's time to sync.
            //
            // Retry policy:
            //  - Never synced (last_sync_at IS NULL) → sync immediately.
            //  - Last sync was "error" or "partial"  → retry after 1 h so a
            //    newly-paired server whose first sync failed (e.g. wrong
            //    address, temporary network issue) is retried well within the
            //    hour rather than waiting the full sync_frequency_hours (24 h
            //    default).  Without this the backup would silently stay empty
            //    for a full day after every failed attempt.
            //  - Last sync succeeded → wait sync_frequency_hours as usual.
            let should_sync = match &server.last_sync_at {
                None => true, // Never synced
                Some(last) => {
                    if let Ok(last_dt) = chrono::DateTime::parse_from_rfc3339(last) {
                        let elapsed = Utc::now() - last_dt.with_timezone(&Utc);
                        let threshold_hours = match server.last_sync_status.as_str() {
                            "error" | "partial" => 1_i64,
                            _ => server.sync_frequency_hours,
                        };
                        elapsed.num_hours() >= threshold_hours
                    } else {
                        true
                    }
                }
            };

            if !should_sync {
                continue;
            }

            // Skip if another sync (manual trigger) is already running for this server
            let guard = match try_acquire_sync(&server.id) {
                Some(g) => g,
                None => {
                    tracing::debug!(
                        server_id = %server.id,
                        "Skipping scheduled sync — another sync is already in progress"
                    );
                    continue;
                }
            };

            let log_id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();

            if let Err(e) = sqlx::query(
                "INSERT INTO backup_sync_log (id, server_id, started_at, status) \
                 VALUES (?, ?, ?, 'running')",
            )
            .bind(&log_id)
            .bind(&server.id)
            .bind(&now)
            .execute(&pool)
            .await
            {
                tracing::warn!(server_id = %server.id, error = %e, "Failed to create backup sync log entry");
            }

            let api_key: Option<String> =
                sqlx::query_scalar("SELECT api_key FROM backup_servers WHERE id = ?")
                    .bind(&server.id)
                    .fetch_optional(&pool)
                    .await
                    .ok()
                    .flatten();

            run_sync(&pool, &storage_root, server, &api_key, &log_id).await;
            drop(guard); // Release the concurrency lock
        }
    }
}

// ── Backup-initiated Sync ────────────────────────────────────────────────────

/// POST /api/backup/request-sync
/// Called by a backup server to request the primary to push data to it.
/// Authenticated via X-API-Key: the primary looks up the backup server by
/// matching the provided key against `backup_servers.api_key`.
pub async fn handle_request_sync(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let provided_key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing X-API-Key header".into()))?;

    // Look up which backup server this API key belongs to
    let server = sqlx::query_as::<_, BackupServer>(
        "SELECT id, name, address, sync_frequency_hours, last_sync_at, \
         last_sync_status, last_sync_error, enabled, created_at \
         FROM backup_servers WHERE api_key = ?",
    )
    .bind(provided_key)
    .fetch_optional(&state.read_pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Unknown API key".into()))?;

    if !server.enabled {
        return Err(AppError::BadRequest("Backup server is disabled".into()));
    }

    // Prevent overlapping syncs to the same server
    let guard = try_acquire_sync(&server.id).ok_or_else(|| {
        AppError::BadRequest("A sync is already in progress for this server".into())
    })?;

    // Create sync log entry
    let log_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO backup_sync_log (id, server_id, started_at, status) VALUES (?, ?, ?, 'running')",
    )
    .bind(&log_id)
    .bind(&server.id)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    // Spawn the sync as a background task
    let pool = state.pool.clone();
    let storage_root = (**state.storage_root.load()).clone();
    let api_key: Option<String> = Some(provided_key.to_string());
    let log_id_clone = log_id.clone();
    let server_name = server.name.clone();

    tokio::spawn(async move {
        let _guard = guard;
        run_sync(&pool, &storage_root, &server, &api_key, &log_id_clone).await;
    });

    tracing::info!(
        server_name = %server_name,
        "Sync requested by backup server"
    );

    Ok(Json(serde_json::json!({
        "message": "Sync started",
        "sync_id": log_id,
    })))
}

/// POST /api/admin/backup/force-sync
/// Admin-only endpoint for backup servers. Contacts the primary server and
/// requests it to push the latest data to this backup instance.
pub async fn force_sync_from_primary(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&state, &auth).await?;

    // This endpoint only makes sense on backup servers
    let mode: String =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_mode'")
            .fetch_optional(&state.read_pool)
            .await?
            .unwrap_or_else(|| "primary".to_string());

    if mode != "backup" {
        return Err(AppError::BadRequest(
            "This endpoint is only available on backup servers".into(),
        ));
    }

    // Get primary server URL and our API key
    let primary_url: String =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'primary_server_url'")
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or_else(|| AppError::BadRequest("No primary server URL configured".into()))?;

    let api_key: String = {
        // Prefer config-file key, fall back to DB-generated key
        if let Some(k) = state
            .config
            .backup
            .api_key
            .as_deref()
            .filter(|k| !k.is_empty())
        {
            k.to_string()
        } else {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT value FROM server_settings WHERE key = 'backup_api_key'",
            )
            .fetch_optional(&state.read_pool)
            .await?
            .flatten()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| AppError::BadRequest("No backup API key configured".into()))?
        }
    };

    // Contact the primary server's request-sync endpoint
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {}", e)))?;

    let url = format!(
        "{}/api/backup/request-sync",
        primary_url.trim_end_matches('/')
    );
    let resp = client
        .post(&url)
        .header("X-API-Key", &api_key)
        .send()
        .await
        .map_err(|e| {
            AppError::Internal(format!(
                "Failed to contact primary server at {}: {}",
                url, e
            ))
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "Primary server returned HTTP {}: {}",
            status,
            body.chars().take(500).collect::<String>()
        )));
    }

    let body: serde_json::Value = resp.json().await.unwrap_or_default();

    tracing::info!(
        "Force sync requested from primary server at {}",
        primary_url
    );

    Ok(Json(serde_json::json!({
        "message": body.get("message").and_then(|m| m.as_str()).unwrap_or("Sync requested"),
        "sync_id": body.get("sync_id").and_then(|s| s.as_str()),
    })))
}
