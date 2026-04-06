//! Core sync engine: orchestrates delta-transfer of photos and trash to a
//! remote backup server.
//!
//! Extracted from [`super::sync`] to keep the HTTP handlers and background
//! scheduler separate from the transfer logic.

use std::collections::{HashMap, HashSet};

use chrono::Utc;

use super::models::BackupServer;
use super::sync_transfer::{
    build_photo_headers, build_trash_headers, fetch_remote_ids, send_file, update_sync_log,
    PhotoToSync, TrashToSync,
};
use super::sync_blobs::sync_blobs;
use super::sync_galleries::sync_secure_galleries_to_backup;
use super::sync_metadata::sync_metadata_to_backup;
use super::sync_users::{sync_user_deletions_to_backup, sync_users_to_backup};

// ── Sync context ─────────────────────────────────────────────────────────────

/// Shared context for all sync sub-phases.
pub(crate) struct SyncContext<'a> {
    pub pool: &'a sqlx::SqlitePool,
    pub client: &'a reqwest::Client,
    pub base_url: String,
    pub api_key: &'a Option<String>,
    pub storage_root: &'a std::path::Path,
    pub server: &'a BackupServer,
}

/// Mutable counters tracking sync progress across phases.
pub(crate) struct SyncCounters {
    pub items_synced: i64,
    pub bytes_synced: i64,
    pub failures: i64,
    pub last_error: Option<String>,
}

impl SyncCounters {
    pub fn new() -> Self {
        Self {
            items_synced: 0,
            bytes_synced: 0,
            failures: 0,
            last_error: None,
        }
    }

    pub fn record_success(&mut self, bytes: i64) {
        self.items_synced += 1;
        self.bytes_synced += bytes;
    }

    pub fn record_failure(&mut self, id: &str, kind: &str, error: String) {
        self.failures += 1;
        tracing::warn!("Backup sync failed for {} {}: {}", kind, id, error);
        self.last_error = Some(error);
    }
}

// ── Sync Engine ──────────────────────────────────────────────────────────────

/// Run the actual sync operation against a backup server.
///
/// **Phase 0a:** Purge photos deleted on the primary from the backup gallery.
/// **Phase 0:** Sync user accounts (must run before photos/trash for FK integrity).
/// **Phase 0b:** Sync user deletions.
/// **Phase 1:** Delta-transfer photos the remote doesn't have.
/// **Phase 2:** Delta-transfer trash items the remote doesn't have.
/// **Phase 3:** Sync secure gallery metadata (full-state JSON).
/// **Phase 4:** Delta-transfer client-encrypted blobs.
/// **Phase 5:** Sync metadata tables — edit_copies, photo_metadata, shared albums (full-state JSON).
///
pub async fn run_sync(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
    server: &BackupServer,
    api_key: &Option<String>,
    log_id: &str,
) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .danger_accept_invalid_certs(true)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            update_sync_log(pool, log_id, "error", 0, 0, Some(&e.to_string())).await;
            return;
        }
    };

    let ctx = SyncContext {
        pool,
        client: &client,
        base_url: format!("http://{}/api", server.address),
        api_key,
        storage_root,
        server,
    };

    let has_key = api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false);
    tracing::info!(
        server_name = %server.name,
        server_address = %server.address,
        base_url = %ctx.base_url,
        has_api_key = has_key,
        log_id = %log_id,
        "Starting sync to backup server"
    );

    // ── Pre-flight: verify the backup server is reachable ───────────────
    if let Err(msg) = preflight_health_check(ctx.client, ctx.server).await {
        update_sync_log(ctx.pool, log_id, "error", 0, 0, Some(&msg)).await;
        update_server_sync_status(ctx.pool, &ctx.server.id, "error", Some(&msg)).await;
        return;
    }

    // ── Delta: fetch IDs the remote already has ──────────────────────────
    let remote_photo_ids: HashSet<String> =
        fetch_remote_ids(ctx.client, &ctx.base_url, "/backup/list", ctx.api_key).await;
    let remote_trash_ids: HashSet<String> =
        fetch_remote_ids(ctx.client, &ctx.base_url, "/backup/list-trash", ctx.api_key).await;
    let remote_blob_ids: HashSet<String> =
        fetch_remote_ids(ctx.client, &ctx.base_url, "/backup/list-blobs", ctx.api_key).await;

    // ── Phase 0a: purge photos deleted on the primary ────────────────────
    purge_deleted_photos_from_backup(&ctx, &remote_photo_ids).await;

    // ── Phase 0: sync user accounts ──────────────────────────────────────
    sync_users_to_backup(ctx.pool, ctx.client, &ctx.base_url, ctx.api_key).await;

    // ── Phase 0b: sync user deletions ────────────────────────────────────
    sync_user_deletions_to_backup(ctx.pool, ctx.client, &ctx.base_url, ctx.api_key, &ctx.server.name).await;

    // ── Phase 1 & 2: transfer photos and trash ───────────────────────────
    let mut counters = SyncCounters::new();

    sync_photos(&ctx, &remote_photo_ids, &mut counters).await;

    // ── Phase 4: sync client-encrypted blobs ─────────────────────────────
    // E2EE data uploaded by clients — irreplaceable, only the client has
    // the decryption key.  Must be backed up for disaster recovery.
    sync_blobs(&ctx, &remote_blob_ids, &mut counters).await;

    // ── Phase 5: sync metadata tables ────────────────────────────────────
    // Lightweight JSON tables: edit_copies, photo_metadata, shared_albums.
    // Full-state sync (not delta) — sent every cycle, backup prunes stale.
    sync_metadata_to_backup(ctx.pool, ctx.client, &ctx.base_url, ctx.api_key).await;
    sync_trash(&ctx, &remote_trash_ids, &mut counters).await;

    // ── Phase 3: sync secure gallery metadata ────────────────────────────
    // Must run AFTER photos so the clone rows exist on the backup before
    // the gallery_items reference them.  This lets the backup's
    // encrypted-sync endpoint correctly filter clones from the gallery.
    sync_secure_galleries_to_backup(ctx.pool, ctx.client, &ctx.base_url, ctx.api_key).await;

    // ── Finalize ─────────────────────────────────────────────────────────
    let (status, error_detail) = if counters.failures == 0 {
        ("success", None)
    } else if counters.items_synced == 0 && counters.failures > 0 {
        (
            "error",
            Some(format!(
                "All {} transfers failed. Last error: {}",
                counters.failures,
                counters.last_error.as_deref().unwrap_or("unknown")
            )),
        )
    } else {
        (
            "partial",
            Some(format!(
                "{} of {} transfers failed. Last error: {}",
                counters.failures,
                counters.items_synced + counters.failures,
                counters.last_error.as_deref().unwrap_or("unknown")
            )),
        )
    };

    update_sync_log(
        ctx.pool,
        log_id,
        status,
        counters.items_synced,
        counters.bytes_synced,
        error_detail.as_deref(),
    )
    .await;

    update_server_sync_status(ctx.pool, &ctx.server.id, status, error_detail.as_deref()).await;

    tracing::info!(
        "Backup sync to '{}' complete: {} synced, {} failed, {} bytes",
        ctx.server.name,
        counters.items_synced,
        counters.failures,
        counters.bytes_synced
    );
}

// ── Sync sub-phases ──────────────────────────────────────────────────────────

/// Pre-flight health check: verify the backup server is reachable before
/// starting the sync. Returns `Ok(())` or `Err(error_message)`.
async fn preflight_health_check(
    client: &reqwest::Client,
    server: &BackupServer,
) -> Result<(), String> {
    let health_url = format!("http://{}/health", server.address);
    match client.get(&health_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                server = %server.name,
                address = %server.address,
                "Backup server health check passed"
            );
            Ok(())
        }
        Ok(resp) => Err(format!(
            "Backup server health check returned HTTP {} at {}",
            resp.status(),
            health_url
        )),
        Err(e) => Err(format!(
            "Cannot reach backup server at {} — {}\n\
             Hint: verify the registered address in backup_servers matches \
             the backup's externally-reachable host:port (check base_url in \
             the backup's config.toml).",
            health_url, e
        )),
    }
}

/// Purge photos that have been deleted on the primary from the backup's
/// gallery. These are items now in the primary's trash whose photo_id
/// still exists in the remote's photo list.
async fn purge_deleted_photos_from_backup(
    ctx: &SyncContext<'_>,
    _remote_photo_ids: &HashSet<String>,
) {
    // Collect photo_id AND encrypted_blob_id from trash — for encrypted items
    // photo_id is the blob UUID which differs from photos.id on the backup.
    // We send both so the backup can match by either column.
    let primary_trash_ids: Vec<String> = match sqlx::query_scalar::<_, String>(
        "SELECT photo_id FROM trash_items \
         UNION SELECT encrypted_blob_id FROM trash_items WHERE encrypted_blob_id IS NOT NULL",
    )
    .fetch_all(ctx.pool)
    .await
    {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("Failed to fetch primary trash photo_ids for deletion sync: {}", e);
            return;
        }
    };

    // Don't filter by remote_photo_ids — encrypted items have photo_id =
    // blob_id which won't appear in remote_photo_ids (which lists photos.id).
    // Sending extra IDs is harmless (DELETE is a no-op for non-matches).
    let to_delete: Vec<&String> = primary_trash_ids
        .iter()
        .filter(|id| !id.is_empty())
        .collect();

    if to_delete.is_empty() {
        return;
    }

    let payload = serde_json::json!({ "deleted_ids": to_delete });
    let url = format!("{}/backup/sync-deletions", ctx.base_url);
    let mut req = ctx.client.post(&url).json(&payload);
    if let Some(ref key) = ctx.api_key {
        req = req.header("X-API-Key", key.as_str());
    }
    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                server = %ctx.server.name,
                count = to_delete.len(),
                "Purged deleted photos from backup gallery"
            );
        }
        Ok(resp) => {
            tracing::warn!(
                server = %ctx.server.name,
                status = %resp.status(),
                "sync-deletions returned non-success status"
            );
        }
        Err(e) => {
            tracing::warn!(
                server = %ctx.server.name,
                "sync-deletions request failed: {}",
                e
            );
        }
    }
}

/// Phase 1: delta-transfer photos the remote doesn't have.
async fn sync_photos(
    ctx: &SyncContext<'_>,
    remote_photo_ids: &HashSet<String>,
    counters: &mut SyncCounters,
) {
    // Pre-fetch ALL photo tags to avoid N+1 inside the transfer loop
    let all_tags = fetch_all_photo_tags(ctx.pool).await;

    // Sync ALL registered media — including audio. The audio_backup_enabled
    // setting only controls whether new audio files are *registered* during
    // auto-scan, not whether already-registered files are transferred.
    // Exclude secure-gallery clones: the `encrypted_gallery_items` table
    // tracks cloned blob_ids and the original_blob_ids they shadow.  The
    // primary's gallery-listing endpoints already filter these out; the
    // sync engine must do the same so the backup never receives clone rows
    // that would appear as duplicates.
    let photos: Vec<PhotoToSync> = {
        let query =
            "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, taken_at, latitude, longitude, \
             width, height, duration_secs, camera_model, is_favorite, photo_hash, \
             crop_metadata, created_at FROM photos \
             WHERE id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
               AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL)";
        match sqlx::query_as::<_, PhotoToSync>(query)
            .fetch_all(ctx.pool)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to query photos for sync: {}", e);
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
        ctx.server.name,
        photos_to_sync.len(),
        photos.len()
    );

    for photo in &photos_to_sync {
        let tags = all_tags.get(&photo.id).cloned().unwrap_or_default();
        let headers = build_photo_headers(photo, &tags);
        match send_file(
            ctx.client,
            &ctx.base_url,
            ctx.api_key,
            ctx.storage_root,
            &photo.id,
            &photo.file_path,
            "photos",
            &headers,
        )
        .await
        {
            Ok(()) => counters.record_success(photo.size_bytes),
            Err(e) => counters.record_failure(&photo.id, "photo", e),
        }
    }
}

/// Phase 2: delta-transfer trash items the remote doesn't have.
async fn sync_trash(
    ctx: &SyncContext<'_>,
    remote_trash_ids: &HashSet<String>,
    counters: &mut SyncCounters,
) {
    let trash_items: Vec<TrashToSync> = match sqlx::query_as::<_, TrashToSync>(
        "SELECT id, photo_id, user_id, filename, file_path, mime_type, media_type, size_bytes, taken_at, latitude, longitude, \
         width, height, duration_secs, camera_model, is_favorite, photo_hash, \
         crop_metadata, deleted_at, expires_at FROM trash_items",
    )
    .fetch_all(ctx.pool)
    .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to query trash items for sync: {}", e);
            return;
        }
    };

    let trash_to_sync: Vec<_> = trash_items
        .iter()
        .filter(|t| !remote_trash_ids.contains(&t.id))
        .collect();

    tracing::info!(
        "Backup sync to '{}': {}/{} trash items need transfer (delta)",
        ctx.server.name,
        trash_to_sync.len(),
        trash_items.len()
    );

    for item in &trash_to_sync {
        let headers = build_trash_headers(item);
        match send_file(
            ctx.client,
            &ctx.base_url,
            ctx.api_key,
            ctx.storage_root,
            &item.id,
            &item.file_path,
            "trash",
            &headers,
        )
        .await
        {
            Ok(()) => counters.record_success(item.size_bytes),
            Err(e) => counters.record_failure(&item.id, "trash", e),
        }
    }
}

// ── Small helpers ────────────────────────────────────────────────────────────

/// Pre-fetch all photo tags in one query (avoids N+1 in the transfer loop).
async fn fetch_all_photo_tags(
    pool: &sqlx::SqlitePool,
) -> HashMap<String, Vec<String>> {
    match sqlx::query_as::<_, (String, String)>(
        "SELECT photo_id, tag FROM photo_tags ORDER BY photo_id",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => {
            let mut map = HashMap::new();
            for (photo_id, tag) in rows {
                map.entry(photo_id).or_insert_with(Vec::new).push(tag);
            }
            map
        }
        Err(e) => {
            tracing::warn!("Failed to fetch photo tags for sync: {}", e);
            HashMap::new()
        }
    }
}

/// Update the backup_servers row with the latest sync status.
pub(crate) async fn update_server_sync_status(
    pool: &sqlx::SqlitePool,
    server_id: &str,
    status: &str,
    error: Option<&str>,
) {
    let now = Utc::now().to_rfc3339();
    if let Err(e) = sqlx::query(
        "UPDATE backup_servers SET last_sync_at = ?, last_sync_status = ?, \
         last_sync_error = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(status)
    .bind(error)
    .bind(server_id)
    .execute(pool)
    .await
    {
        tracing::warn!(server_id = %server_id, error = %e, "Failed to update backup server sync status");
    }
}
