//! Client-encrypted blob synchronization from primary → backup server.
//!
//! Transfers the actual encrypted blob files (E2EE data uploaded by clients)
//! along with their metadata.  These are critical for recovery since only the
//! client holds the decryption key — the server cannot recreate them.

use std::collections::HashSet;

use super::sync_engine::{SyncContext, SyncCounters};
use super::sync_transfer::{build_blob_headers, send_blob_to_backup, BlobToSync};

/// Phase 4: delta-transfer blobs the remote doesn't have.
/// Excludes secure-gallery clones that have been server-side encrypted
/// (the encrypted version is synced instead). Client-encrypted clone blobs
/// (no photos row with encrypted_blob_id) are included so the backup can
/// serve them directly. Also excludes original_blob_ids and the originals'
/// encrypted blobs/thumbs. Blobs with missing backing files are silently
/// skipped (handles gallery-placeholder blobs on backup during recovery push).
pub async fn sync_blobs(
    ctx: &SyncContext<'_>,
    remote_blob_ids: &HashSet<String>,
    counters: &mut SyncCounters,
) {
    let blobs: Vec<BlobToSync> = match sqlx::query_as::<_, BlobToSync>(
        "SELECT id, user_id, blob_type, size_bytes, client_hash, upload_time, \
         storage_path, content_hash FROM blobs \
         WHERE id NOT IN ( \
               SELECT gi.blob_id FROM encrypted_gallery_items gi \
               INNER JOIN photos p ON p.id = gi.blob_id AND p.encrypted_blob_id IS NOT NULL) \
           AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
           AND id NOT IN ( \
               SELECT p.encrypted_blob_id FROM photos p \
               WHERE p.encrypted_blob_id IS NOT NULL \
               AND p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
               AND p.encrypted_blob_id NOT IN ( \
                   SELECT p2.encrypted_blob_id FROM photos p2 \
                   WHERE p2.encrypted_blob_id IS NOT NULL \
                   AND p2.id IN (SELECT blob_id FROM encrypted_gallery_items))) \
           AND id NOT IN ( \
               SELECT p.encrypted_thumb_blob_id FROM photos p \
               WHERE p.encrypted_thumb_blob_id IS NOT NULL \
               AND p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
               AND p.encrypted_thumb_blob_id NOT IN ( \
                   SELECT p2.encrypted_thumb_blob_id FROM photos p2 \
                   WHERE p2.encrypted_thumb_blob_id IS NOT NULL \
                   AND p2.id IN (SELECT blob_id FROM encrypted_gallery_items)))",
    )
    .fetch_all(ctx.pool)
    .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to query blobs for sync: {}", e);
            return;
        }
    };

    // Pre-filter: skip blobs whose backing files are missing on disk.
    // This happens when this server is a backup running recovery push-sync:
    // gallery-placeholder blobs have empty storage_path, and encrypted
    // derivative blobs may have been cleaned up by gallery sync purge.
    let mut skipped_missing = 0usize;
    let mut valid_blobs: Vec<&BlobToSync> = Vec::new();
    for b in &blobs {
        if b.storage_path.is_empty() {
            skipped_missing += 1;
            continue;
        }
        let full_path = ctx.storage_root.join(&b.storage_path);
        if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
            skipped_missing += 1;
            continue;
        }
        valid_blobs.push(b);
    }
    if skipped_missing > 0 {
        tracing::warn!(
            "sync_blobs: skipped {} blobs with missing/empty files out of {} total",
            skipped_missing, blobs.len()
        );
    }

    let to_sync: Vec<_> = valid_blobs
        .into_iter()
        .filter(|b| !remote_blob_ids.contains(&b.id))
        .collect();

    tracing::info!(
        "Backup sync to '{}': {}/{} blobs need transfer (delta)",
        ctx.server.name,
        to_sync.len(),
        blobs.len()
    );

    for blob in &to_sync {
        let headers = build_blob_headers(blob);
        match send_blob_to_backup(
            ctx.client,
            &ctx.base_url,
            ctx.api_key,
            ctx.storage_root,
            blob,
            &headers,
        )
        .await
        {
            Ok(()) => counters.record_success(blob.size_bytes),
            Err(e) => counters.record_failure(&blob.id, "blob", e),
        }
    }
}
