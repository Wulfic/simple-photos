//! Client-encrypted blob synchronization from primary → backup server.
//!
//! Transfers the actual encrypted blob files (E2EE data uploaded by clients)
//! along with their metadata.  These are critical for recovery since only the
//! client holds the decryption key — the server cannot recreate them.

use std::collections::HashSet;

use super::sync_engine::{SyncContext, SyncCounters};
use super::sync_transfer::{build_blob_headers, send_blob_to_backup, BlobToSync};

/// Phase 4: delta-transfer blobs the remote doesn't have.
/// Excludes secure-gallery clones — both the clone blob_id and the original
/// blob_id that it shadows are filtered out, matching the primary's
/// GET /api/blobs listing behaviour.
pub async fn sync_blobs(
    ctx: &SyncContext<'_>,
    remote_blob_ids: &HashSet<String>,
    counters: &mut SyncCounters,
) {
    let blobs: Vec<BlobToSync> = match sqlx::query_as::<_, BlobToSync>(
        "SELECT id, user_id, blob_type, size_bytes, client_hash, upload_time, \
         storage_path, content_hash FROM blobs \
         WHERE id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
           AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
           AND id NOT IN ( \
               SELECT p.encrypted_blob_id FROM photos p \
               WHERE p.encrypted_blob_id IS NOT NULL \
               AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                    OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
           AND id NOT IN ( \
               SELECT p.encrypted_thumb_blob_id FROM photos p \
               WHERE p.encrypted_thumb_blob_id IS NOT NULL \
               AND (p.id IN (SELECT blob_id FROM encrypted_gallery_items) \
                    OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL)))",
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

    let to_sync: Vec<_> = blobs
        .iter()
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
