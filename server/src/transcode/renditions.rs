//! Persistence for the video resolution ladder (#49).
//!
//! [`ladder`](super::ladder) decides *which* renditions a source should have;
//! this module records the ones that were actually produced and hands them back
//! to the picker.
//!
//! Storage mode mirrors the parent photo: `blob_id` in encrypted mode (which is
//! what both clients actually play from — see `035_video_renditions.sql`),
//! `file_path` when the server is running unencrypted. See that migration for
//! why the plaintext-file-only design does not work.

use serde::Serialize;

use crate::error::AppError;

/// One stored rendition of a video.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct StoredRendition {
    pub photo_id: String,
    pub short_edge: i64,
    pub width: i64,
    pub height: i64,
    /// SQLite has no bool; 0/1.
    pub is_source: i64,
    pub blob_id: Option<String>,
    pub file_path: Option<String>,
    pub codec: Option<String>,
    pub bitrate: Option<i64>,
    pub size_bytes: i64,
}

impl StoredRendition {
    pub fn is_source(&self) -> bool {
        self.is_source != 0
    }

    /// Whether this rendition has bytes a client could actually fetch.
    ///
    /// A row with neither locator is *planned but not produced* — the ladder
    /// recorded the intent and the encode has not finished (or failed). Serving
    /// such a row to a picker offers a quality that 404s.
    pub fn is_playable(&self) -> bool {
        self.blob_id.is_some() || self.file_path.is_some()
    }
}

/// Record (or update) one rendition.
///
/// Upsert rather than insert: re-running the ladder over a photo — a backfill
/// pass, a re-encode after a failure — must refresh a rung in place. The
/// primary key is `(photo_id, short_edge)`, so a duplicate rung is impossible
/// by construction rather than by discipline at the call sites.
///
/// Clears `not_needed` (037): producing bytes for a rung settles the question
/// of whether it was owed, whatever an earlier probe concluded. Leaving the two
/// set at once would be a row that claims both "here are the bytes" and "no
/// bytes are required", and the next reader would have to guess which is meant.
pub async fn upsert_rendition(
    pool: &sqlx::SqlitePool,
    r: &StoredRendition,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO video_renditions \
         (photo_id, short_edge, width, height, is_source, blob_id, file_path, \
          codec, bitrate, size_bytes, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now')) \
         ON CONFLICT(photo_id, short_edge) DO UPDATE SET \
           width = excluded.width, height = excluded.height, \
           is_source = excluded.is_source, blob_id = excluded.blob_id, \
           file_path = excluded.file_path, codec = excluded.codec, \
           bitrate = excluded.bitrate, size_bytes = excluded.size_bytes, \
           not_needed = 0",
    )
    .bind(&r.photo_id)
    .bind(r.short_edge)
    .bind(r.width)
    .bind(r.height)
    .bind(r.is_source)
    .bind(&r.blob_id)
    .bind(&r.file_path)
    .bind(&r.codec)
    .bind(r.bitrate)
    .bind(r.size_bytes)
    .execute(pool)
    .await
    .map_err(|e| {
        // Every failure path logs — a rendition that silently fails to record
        // is a transcode's worth of CPU thrown away with no trace.
        tracing::error!(
            photo_id = %r.photo_id,
            short_edge = r.short_edge,
            "failed to record video rendition: {e}"
        );
        AppError::from(e)
    })?;
    Ok(())
}

/// Every rendition of a photo, highest quality first — the order a picker
/// displays and the order a "default to highest" client reads.
///
/// Unproduced rungs are filtered out here rather than at each call site: a
/// picker must never offer a quality that does not exist.
pub async fn list_renditions(
    pool: &sqlx::SqlitePool,
    photo_id: &str,
) -> Result<Vec<StoredRendition>, AppError> {
    let rows: Vec<StoredRendition> = sqlx::query_as(
        "SELECT photo_id, short_edge, width, height, is_source, blob_id, \
                file_path, codec, bitrate, size_bytes \
         FROM video_renditions WHERE photo_id = ? ORDER BY short_edge DESC",
    )
    .bind(photo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!(photo_id = %photo_id, "failed to list video renditions: {e}");
        AppError::from(e)
    })?;

    Ok(rows.into_iter().filter(StoredRendition::is_playable).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// Foreign keys ON — unlike most harnesses in this crate, because the whole
    /// point of these tests is the ON DELETE CASCADE path.
    async fn test_pool() -> sqlx::SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at) \
             VALUES ('u1', 'u1', 'x', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn insert_photo(pool: &sqlx::SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, \
             media_type, size_bytes, width, height, created_at) \
             VALUES (?, 'u1', ?, ?, 'video/mp4', 'video', 0, 3840, 2160, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("{id}.mp4"))
        .bind(format!("uploads/{id}.mp4"))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_blob(pool: &sqlx::SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
             VALUES (?, 'u1', 'photo', 1024, '2026-01-01T00:00:00Z', ?)",
        )
        .bind(id)
        .bind(format!("blobs/u1/{id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    fn rendition(photo_id: &str, short_edge: i64, blob: Option<&str>) -> StoredRendition {
        StoredRendition {
            photo_id: photo_id.into(),
            short_edge,
            width: 1920,
            height: short_edge,
            is_source: 0,
            blob_id: blob.map(str::to_string),
            file_path: None,
            codec: Some("h264".into()),
            bitrate: Some(4_000_000),
            size_bytes: 1024,
        }
    }

    async fn head_seq(pool: &sqlx::SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM photo_change_log")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Re-running the ladder must refresh a rung, never accumulate duplicates —
    /// a backfill pass over a library that already has renditions is the normal
    /// case, not an edge case.
    #[tokio::test]
    async fn re_running_the_ladder_updates_a_rung_in_place() {
        let pool = test_pool().await;
        insert_photo(&pool, "p1").await;
        insert_blob(&pool, "b1").await;
        insert_blob(&pool, "b2").await;

        upsert_rendition(&pool, &rendition("p1", 1080, Some("b1")))
            .await
            .unwrap();
        let mut second = rendition("p1", 1080, Some("b2"));
        second.size_bytes = 4096;
        upsert_rendition(&pool, &second).await.unwrap();

        let got = list_renditions(&pool, "p1").await.unwrap();
        assert_eq!(got.len(), 1, "the same rung must not appear twice");
        assert_eq!(got[0].blob_id.as_deref(), Some("b2"));
        assert_eq!(got[0].size_bytes, 4096);
    }

    /// Highest first — the picker's display order and the "default to highest"
    /// client's read order.
    #[tokio::test]
    async fn renditions_come_back_highest_first() {
        let pool = test_pool().await;
        insert_photo(&pool, "p1").await;
        insert_blob(&pool, "b1").await;
        insert_blob(&pool, "b2").await;

        upsert_rendition(&pool, &rendition("p1", 1080, Some("b1")))
            .await
            .unwrap();
        let mut source = rendition("p1", 2160, Some("b2"));
        source.is_source = 1;
        upsert_rendition(&pool, &source).await.unwrap();

        let got = list_renditions(&pool, "p1").await.unwrap();
        assert_eq!(
            got.iter().map(|r| r.short_edge).collect::<Vec<_>>(),
            vec![2160, 1080]
        );
        assert!(got[0].is_source());
    }

    /// A rung whose encode has not produced bytes must not reach a picker —
    /// offering it yields a quality option that 404s.
    #[tokio::test]
    async fn planned_but_unproduced_rungs_are_not_offered() {
        let pool = test_pool().await;
        insert_photo(&pool, "p1").await;
        insert_blob(&pool, "b1").await;

        upsert_rendition(&pool, &rendition("p1", 1080, None))
            .await
            .unwrap();
        assert!(
            list_renditions(&pool, "p1").await.unwrap().is_empty(),
            "a rendition with no blob and no file has nothing to serve"
        );

        upsert_rendition(&pool, &rendition("p1", 1080, Some("b1")))
            .await
            .unwrap();
        assert_eq!(list_renditions(&pool, "p1").await.unwrap().len(), 1);
    }

    /// The property the whole GC design rests on. Deleting a photo cascades its
    /// rendition rows away, and the AFTER DELETE trigger on that cascade is the
    /// only thing that records the now-unreferenced video-sized blob.
    ///
    /// If this breaks, every deleted 4K video silently leaks its rendition
    /// bytes — invisibly, at hundreds of MB each.
    #[tokio::test]
    async fn deleting_a_photo_cascades_and_records_the_orphaned_blob() {
        let pool = test_pool().await;
        insert_photo(&pool, "p1").await;
        insert_blob(&pool, "b1").await;
        upsert_rendition(&pool, &rendition("p1", 1080, Some("b1")))
            .await
            .unwrap();

        sqlx::query("DELETE FROM photos WHERE id = 'p1'")
            .execute(&pool)
            .await
            .unwrap();

        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM video_renditions WHERE photo_id = 'p1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 0, "renditions must not outlive their photo");

        let orphans: Vec<(String, String)> =
            sqlx::query_as("SELECT blob_id, user_id FROM orphaned_rendition_blobs")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            orphans,
            vec![("b1".to_string(), "u1".to_string())],
            "the cascade must queue the blob for sweeping, with a user_id the \
             sweeper can still resolve a path from"
        );
    }

    /// **A source rendition never owns its bytes**, so deleting one must never
    /// queue them for collection.
    ///
    /// The generation pass records a source rung pointing at the photo's own
    /// `encrypted_blob_id` — a second reference to bytes the photo still owns,
    /// not a copy of them. `035`'s trigger predates that row and cannot tell the
    /// two cases apart, so it queued the user's ORIGINAL video for the GC sweep.
    ///
    /// The sweeper is required to re-check references before unlinking, so this
    /// is not by itself data loss. But that makes the safety of an original 4K
    /// video depend on a sweeper that does not exist yet being careful about a
    /// case its author has to know about. `037` guards the trigger on
    /// `is_source = 0` so the queue can only ever name bytes a rendition owns.
    ///
    /// Verified RED against `035`'s unguarded trigger: `b1` is queued.
    #[tokio::test]
    async fn deleting_a_source_rendition_never_queues_the_photos_own_blob() {
        let pool = test_pool().await;
        insert_photo(&pool, "p1").await;
        insert_blob(&pool, "b1").await;

        let mut source = rendition("p1", 2160, Some("b1"));
        source.is_source = 1;
        upsert_rendition(&pool, &source).await.unwrap();

        sqlx::query("DELETE FROM photos WHERE id = 'p1'")
            .execute(&pool)
            .await
            .unwrap();

        let orphans: Vec<(String,)> =
            sqlx::query_as("SELECT blob_id FROM orphaned_rendition_blobs")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(
            orphans.is_empty(),
            "a source rung shares the photo's blob; queueing it points the sweeper \
             at the user's original video, got {orphans:?}"
        );
    }

    /// Renditions land minutes-to-hours after the photo, by which time every
    /// client has synced it and — post-#38 — will never ask about it again
    /// unless the change log nominates it.
    #[tokio::test]
    async fn a_new_rendition_nominates_its_photo_for_re_sync() {
        let pool = test_pool().await;
        insert_photo(&pool, "p1").await;
        insert_blob(&pool, "b1").await;
        let before = head_seq(&pool).await;

        upsert_rendition(&pool, &rendition("p1", 1080, Some("b1")))
            .await
            .unwrap();

        let after = head_seq(&pool).await;
        assert!(
            after > before,
            "a new rung must move head_seq ({before} -> {after}), or the picker \
             stays empty until a full walk that may never come"
        );
        let nominated: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM photo_change_log WHERE photo_id = 'p1' AND user_id = 'u1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(nominated, 1);
    }

    /// Withdrawing a rung changes the picker as much as adding one does.
    #[tokio::test]
    async fn withdrawing_a_rendition_also_nominates_the_photo() {
        let pool = test_pool().await;
        insert_photo(&pool, "p1").await;
        insert_blob(&pool, "b1").await;
        upsert_rendition(&pool, &rendition("p1", 1080, Some("b1")))
            .await
            .unwrap();
        let before = head_seq(&pool).await;

        sqlx::query("DELETE FROM video_renditions WHERE photo_id = 'p1'")
            .execute(&pool)
            .await
            .unwrap();

        assert!(head_seq(&pool).await > before);
    }

    /// The delete trigger writes a change-log row by SELECTing the photo. On the
    /// cascade path the photo is already gone, so that SELECT matches nothing
    /// and `trg_photo_change_log_delete` owns the tombstone. Pin it: a resurrected
    /// change-log entry naming a deleted photo would hand clients a row the
    /// delta feed then has to re-derive away.
    #[tokio::test]
    async fn the_cascade_does_not_resurrect_a_change_log_row_for_a_deleted_photo() {
        let pool = test_pool().await;
        insert_photo(&pool, "p1").await;
        insert_blob(&pool, "b1").await;
        upsert_rendition(&pool, &rendition("p1", 1080, Some("b1")))
            .await
            .unwrap();

        sqlx::query("DELETE FROM photos WHERE id = 'p1'")
            .execute(&pool)
            .await
            .unwrap();

        // Exactly one row, and it is the tombstone written by the photos
        // trigger — not a duplicate from the rendition cascade.
        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM photo_change_log WHERE photo_id = 'p1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 1);
        let still_a_photo: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE id = 'p1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(still_a_photo, 0);
    }
}
