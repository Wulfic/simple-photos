//! Reclaim the bytes of deleted video renditions (#49).
//!
//! Deleting a photo cascades its `video_renditions` rows away, and migration
//! `035`'s `trg_video_rendition_blob_orphaned` trigger records each vacated
//! `blob_id` in `orphaned_rendition_blobs` (migration `037` narrowed it to
//! `is_source = 0`, so the queue can only ever name bytes a *rendition* owns —
//! never the user's original video, which a source rung merely second-references).
//!
//! A DB cascade cannot unlink a file or drop the `blobs` row, so those bytes —
//! for the 4K sources this feature targets, hundreds of megabytes each — leak
//! forever until something drains the queue. This is that something.
//!
//! # The queue row is a hint, not a fact
//!
//! Exactly as with #38's tombstones: the trigger writes "these bytes are
//! *probably* unreferenced", never a claim. So the sweeper re-derives the truth
//! from the live tables before unlinking — a re-plan may have reused the blob,
//! and `blobs.content_hash` dedup means one blob can back several photos. An
//! over-fired hint therefore costs one lookup and can never delete live data;
//! an under-fired one is impossible, because a trigger covers every delete site.

use std::path::Path;

use crate::error::AppError;

/// What one drain pass did. `deleted` reclaimed bytes; `still_referenced` were
/// hints that turned out to still be live (dropped without touching the blob).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OrphanSweepOutcome {
    pub examined: usize,
    pub deleted: usize,
    pub still_referenced: usize,
    pub failed: usize,
}

/// Bounds one pass so a boot after a large multi-video delete has a predictable
/// cost; the next sweep drains the rest.
const ORPHAN_SWEEP_LIMIT: i64 = 200;

/// Drain up to [`ORPHAN_SWEEP_LIMIT`] queued orphan blobs, unlinking the ones
/// nothing references any more. Never returns an error — this is background
/// maintenance and a failure to reclaim one blob must not abort the pass or the
/// scan that triggered it. Concurrency-safe: every mutation is idempotent
/// (`delete_blob` tolerates a missing file, and the `DELETE`s no-op on a row a
/// racing pass already removed).
pub async fn sweep_orphaned_rendition_blobs(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
) -> OrphanSweepOutcome {
    let mut outcome = OrphanSweepOutcome::default();

    let queued: Vec<(String, String)> = match sqlx::query_as(
        "SELECT blob_id, user_id FROM orphaned_rendition_blobs \
         ORDER BY detected_at ASC LIMIT ?",
    )
    .bind(ORPHAN_SWEEP_LIMIT)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("[LADDER-GC] failed to read orphan queue: {e}");
            return outcome;
        }
    };

    for (blob_id, _user_id) in queued {
        outcome.examined += 1;
        match reclaim_one(pool, storage_root, &blob_id).await {
            Ok(true) => outcome.deleted += 1,
            Ok(false) => outcome.still_referenced += 1,
            Err(e) => {
                outcome.failed += 1;
                tracing::error!(blob_id = %blob_id, "[LADDER-GC] failed to reclaim orphan: {e}");
            }
        }
    }

    if outcome.examined > 0 {
        tracing::info!(
            examined = outcome.examined,
            deleted = outcome.deleted,
            still_referenced = outcome.still_referenced,
            failed = outcome.failed,
            "[LADDER-GC] rendition orphan sweep complete"
        );
    }

    outcome
}

/// Reclaim one queued blob. Returns `Ok(true)` if bytes were freed, `Ok(false)`
/// if a live reference re-appeared (the hint is dropped, the blob kept).
async fn reclaim_one(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    blob_id: &str,
) -> Result<bool, AppError> {
    // Re-derive the truth. `?1` is reused throughout; SQLite numbered params
    // bind once. A blob is reclaimable only if NOTHING points at it: not another
    // rendition (a re-plan may have reused it), not a photo (content_hash dedup
    // can bind a blob to several), and not a secure-gallery item.
    let still_referenced: bool = sqlx::query_scalar(
        "SELECT \
             EXISTS(SELECT 1 FROM video_renditions WHERE blob_id = ?1) \
          OR EXISTS(SELECT 1 FROM photos WHERE encrypted_blob_id = ?1 \
                       OR encrypted_thumb_blob_id = ?1 OR motion_video_blob_id = ?1) \
          OR EXISTS(SELECT 1 FROM encrypted_gallery_items WHERE blob_id = ?1 \
                       OR original_blob_id = ?1 OR encrypted_blob_id = ?1 \
                       OR encrypted_thumb_blob_id = ?1)",
    )
    .bind(blob_id)
    .fetch_one(pool)
    .await?;

    if still_referenced {
        // Stale hint: drop the queue row, leave the bytes alone.
        sqlx::query("DELETE FROM orphaned_rendition_blobs WHERE blob_id = ?")
            .bind(blob_id)
            .execute(pool)
            .await?;
        return Ok(false);
    }

    // Truly unreferenced. Unlink the file first (idempotent on a missing file),
    // then drop the `blobs` row and the queue row atomically.
    if let Some(storage_path) =
        sqlx::query_scalar::<_, String>("SELECT storage_path FROM blobs WHERE id = ?")
            .bind(blob_id)
            .fetch_optional(pool)
            .await?
    {
        crate::blobs::storage::delete_blob(storage_root, &storage_path).await?;
    }

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM blobs WHERE id = ?")
        .bind(blob_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM orphaned_rendition_blobs WHERE blob_id = ?")
        .bind(blob_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    async fn test_pool() -> sqlx::SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// A unique scratch dir under the OS temp root, holding a fake blob file at
    /// `blobs/<id>`. Returned as (root, relative_path).
    fn scratch_blob(id: &str) -> (std::path::PathBuf, String) {
        let root = std::env::temp_dir().join(format!("orphan-sweep-{}", uuid::Uuid::new_v4()));
        let rel = format!("blobs/{id}");
        let full = root.join(&rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, b"rendition-bytes").unwrap();
        (root, rel)
    }

    async fn insert_blob(pool: &sqlx::SqlitePool, id: &str, storage_path: &str) {
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
             VALUES (?, 'u1', 'rendition', 15, '2026-01-01T00:00:00Z', ?)",
        )
        .bind(id)
        .bind(storage_path)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn queue_orphan(pool: &sqlx::SqlitePool, blob_id: &str) {
        sqlx::query(
            "INSERT INTO orphaned_rendition_blobs (blob_id, user_id, detected_at) \
             VALUES (?, 'u1', '2026-01-01T00:00:00Z')",
        )
        .bind(blob_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn queue_count(pool: &sqlx::SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM orphaned_rendition_blobs")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn blob_exists(pool: &sqlx::SqlitePool, id: &str) -> bool {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        n > 0
    }

    /// The happy path: a queued blob nothing references is unlinked from disk,
    /// its `blobs` row dropped, and the queue row cleared.
    #[tokio::test]
    async fn reclaims_a_truly_orphaned_blob() {
        let pool = test_pool().await;
        let (root, rel) = scratch_blob("orphan-1");
        insert_blob(&pool, "orphan-1", &rel).await;
        queue_orphan(&pool, "orphan-1").await;

        let outcome = sweep_orphaned_rendition_blobs(&pool, &root).await;

        assert_eq!(
            outcome,
            OrphanSweepOutcome {
                examined: 1,
                deleted: 1,
                ..Default::default()
            }
        );
        assert!(
            !root.join(&rel).exists(),
            "the rendition file must be unlinked"
        );
        assert!(
            !blob_exists(&pool, "orphan-1").await,
            "the blobs row must be gone"
        );
        assert_eq!(queue_count(&pool).await, 0, "the queue row must be cleared");

        std::fs::remove_dir_all(&root).ok();
    }

    /// The safety property: a hint whose blob is STILL referenced by a live
    /// rendition (a re-plan reused it) must not delete the bytes. The stale hint
    /// is dropped so it is not re-examined forever.
    #[tokio::test]
    async fn keeps_a_blob_a_rendition_still_references() {
        let pool = test_pool().await;
        let (root, rel) = scratch_blob("shared-1");
        insert_blob(&pool, "shared-1", &rel).await;
        queue_orphan(&pool, "shared-1").await;
        // A live rendition of some other photo still points at this blob.
        sqlx::query(
            "INSERT INTO video_renditions (photo_id, short_edge, width, height, is_source, \
             blob_id, size_bytes, created_at) \
             VALUES ('other-photo', 1080, 1920, 1080, 0, 'shared-1', 15, '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = sweep_orphaned_rendition_blobs(&pool, &root).await;

        assert_eq!(
            outcome,
            OrphanSweepOutcome {
                examined: 1,
                still_referenced: 1,
                ..Default::default()
            }
        );
        assert!(
            root.join(&rel).exists(),
            "referenced bytes must NOT be unlinked"
        );
        assert!(
            blob_exists(&pool, "shared-1").await,
            "referenced blob must survive"
        );
        assert_eq!(
            queue_count(&pool).await,
            0,
            "the stale hint must be dropped"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// content_hash dedup can bind a rendition-shaped blob to a real photo. If a
    /// photo references it, it is live and must survive.
    #[tokio::test]
    async fn keeps_a_blob_a_photo_still_references() {
        let pool = test_pool().await;
        let (root, rel) = scratch_blob("photo-blob");
        insert_blob(&pool, "photo-blob", &rel).await;
        queue_orphan(&pool, "photo-blob").await;
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, mime_type, created_at, encrypted_blob_id) \
             VALUES ('p1', 'u1', 'v.mp4', 'video/mp4', '2026-01-01T00:00:00Z', 'photo-blob')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = sweep_orphaned_rendition_blobs(&pool, &root).await;

        assert_eq!(outcome.still_referenced, 1);
        assert!(blob_exists(&pool, "photo-blob").await);
        std::fs::remove_dir_all(&root).ok();
    }

    /// The whole production chain: deleting a photo cascades its rendition away,
    /// the `035`/`037` trigger queues the vacated blob, and the sweep reclaims
    /// it. Pins that the AFTER DELETE trigger fires on a cascade — the same
    /// SQLite behaviour #38 relies on.
    #[tokio::test]
    async fn photo_delete_cascade_enqueues_then_sweep_reclaims() {
        let pool = test_pool().await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();

        let (root, rel) = scratch_blob("rung-blob");
        // Parents the FK graph needs, then a video with a downscale rendition.
        sqlx::query("INSERT INTO users (id, username, password_hash, created_at) VALUES ('u1','u','h','2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();
        insert_blob(&pool, "rung-blob", &rel).await;
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, mime_type, media_type, created_at) \
             VALUES ('vid', 'u1', 'v.mp4', 'video/mp4', 'video', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO video_renditions (photo_id, short_edge, width, height, is_source, \
             blob_id, size_bytes, created_at) \
             VALUES ('vid', 1080, 1920, 1080, 0, 'rung-blob', 15, '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Delete the photo -> cascade removes the rendition -> trigger enqueues.
        sqlx::query("DELETE FROM photos WHERE id = 'vid'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            queue_count(&pool).await,
            1,
            "cascade must fire the orphan trigger"
        );

        let outcome = sweep_orphaned_rendition_blobs(&pool, &root).await;

        assert_eq!(outcome.deleted, 1);
        assert!(
            !root.join(&rel).exists(),
            "leaked rendition bytes must be freed"
        );
        assert!(!blob_exists(&pool, "rung-blob").await);
        assert_eq!(queue_count(&pool).await, 0);

        std::fs::remove_dir_all(&root).ok();
    }
}
