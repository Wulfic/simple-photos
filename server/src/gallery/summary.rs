//! Precomputed gallery count summary.
//!
//! Historically every client derived its smart-album counts by paginating the
//! **entire** `encrypted-sync` endpoint and counting locally — and the web UI
//! gated display on that full round-trip completing, so opening Albums showed a
//! spinner while the whole library re-synced *every time*. That's the "it
//! recounts every photo for every client" the user reported.
//!
//! `GET /api/photos/summary` returns all the counts a client needs in **one
//! cheap aggregate round-trip**, so smart-album counts render instantly without
//! any gallery pagination. The counts mirror the `encrypted-sync` eligibility
//! filter exactly (same rows the grid would show), including burst collapse.
//!
//! Results are cached per-user in [`SummaryCache`] with a short TTL so a burst
//! of clients (or the 2 s web poll) collapses to a single query, while still
//! reflecting new imports/trashes within a few seconds without any fragile
//! per-write-site invalidation.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// Precomputed gallery counts for a single user. All counts reflect the same
/// rows `encrypted-sync` returns (secure-gallery items excluded).
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct PhotoSummary {
    /// Total media rows (no burst collapse).
    pub total: i64,
    /// Total as the grid shows it: burst frames collapse to one tile.
    pub collapsed_total: i64,
    pub photos: i64,
    pub gifs: i64,
    pub videos: i64,
    pub audio: i64,
    pub favorites: i64,
}

/// The grid collapses every set of frames sharing a `burst_id` into a single
/// tile. Collapsed total = non-burst rows + number of distinct burst groups.
/// Pulled out as a pure function so the arithmetic is unit-testable in isolation.
pub fn collapsed_total(non_burst: i64, burst_groups: i64) -> i64 {
    non_burst + burst_groups
}

/// Per-user, TTL-bounded cache of [`PhotoSummary`]. Lives in [`AppState`].
pub struct SummaryCache {
    inner: Mutex<HashMap<String, (Instant, PhotoSummary)>>,
    ttl: Duration,
}

impl SummaryCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Returns a cached summary for `user_id` if one exists and is younger than
    /// the TTL, else `None`.
    pub fn get_fresh(&self, user_id: &str) -> Option<PhotoSummary> {
        let guard = self.inner.lock().ok()?;
        let (at, summary) = guard.get(user_id)?;
        if at.elapsed() < self.ttl {
            Some(summary.clone())
        } else {
            None
        }
    }

    pub fn put(&self, user_id: &str, summary: PhotoSummary) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(user_id.to_string(), (Instant::now(), summary));
        }
    }

    /// Drop any cached entry for `user_id`. Call after a write (import, trash,
    /// favorite toggle) when you want the next read to recompute immediately
    /// rather than waiting out the TTL.
    pub fn invalidate(&self, user_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.remove(user_id);
        }
    }
}

impl Default for SummaryCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(15))
    }
}

/// Compute the summary directly from the DB (no cache). Mirrors the
/// `encrypted-sync` eligibility filter so counts equal what the grid renders.
pub async fn compute_summary(
    pool: &sqlx::SqlitePool,
    user_id: &str,
) -> Result<PhotoSummary, AppError> {
    // One pass over the eligible rows produces every count. `COUNT(DISTINCT
    // burst_id)` ignores NULLs in SQLite, so it counts only real burst groups.
    let row: (i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
           COUNT(*), \
           COALESCE(SUM(CASE WHEN media_type = 'photo' THEN 1 ELSE 0 END), 0), \
           COALESCE(SUM(CASE WHEN media_type = 'gif'   THEN 1 ELSE 0 END), 0), \
           COALESCE(SUM(CASE WHEN media_type = 'video' THEN 1 ELSE 0 END), 0), \
           COALESCE(SUM(CASE WHEN media_type = 'audio' THEN 1 ELSE 0 END), 0), \
           COALESCE(SUM(CASE WHEN is_favorite THEN 1 ELSE 0 END), 0), \
           COALESCE(SUM(CASE WHEN burst_id IS NULL THEN 1 ELSE 0 END), 0), \
           COUNT(DISTINCT burst_id) \
         FROM photos \
         WHERE user_id = ?1 \
           AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
           AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
           AND (encrypted_blob_id IS NULL OR encrypted_blob_id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    let (total, photos, gifs, videos, audio, favorites, non_burst, burst_groups) = row;
    Ok(PhotoSummary {
        total,
        collapsed_total: collapsed_total(non_burst, burst_groups),
        photos,
        gifs,
        videos,
        audio,
        favorites,
    })
}

/// GET /api/photos/summary
///
/// Cheap, cached, one-round-trip counts for smart-album badges. Clients render
/// counts from this instantly, before/without any full gallery sync.
pub async fn photos_summary(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<PhotoSummary>, AppError> {
    if let Some(cached) = state.summary_cache.get_fresh(&auth.user_id) {
        return Ok(Json(cached));
    }

    let summary = compute_summary(&state.read_pool, &auth.user_id)
        .await
        .map_err(|e| {
            tracing::error!(user_id = %auth.user_id, error = ?e, "photos_summary compute failed");
            e
        })?;
    state.summary_cache.put(&auth.user_id, summary.clone());
    Ok(Json(summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapsed_total_adds_non_burst_and_groups() {
        // 10 loose photos + 2 burst groups (each of many frames) → 12 tiles.
        assert_eq!(collapsed_total(10, 2), 12);
        // No bursts → collapsed == raw.
        assert_eq!(collapsed_total(7, 0), 7);
        // All frames in one burst → a single tile.
        assert_eq!(collapsed_total(0, 1), 1);
    }

    #[test]
    fn cache_respects_ttl_and_invalidation() {
        let cache = SummaryCache::new(Duration::from_millis(50));
        let s = PhotoSummary {
            total: 3,
            ..Default::default()
        };
        assert!(cache.get_fresh("u1").is_none());
        cache.put("u1", s.clone());
        assert_eq!(cache.get_fresh("u1"), Some(s.clone()));

        cache.invalidate("u1");
        assert!(cache.get_fresh("u1").is_none());

        cache.put("u1", s);
        std::thread::sleep(Duration::from_millis(60));
        assert!(cache.get_fresh("u1").is_none(), "entry should expire after TTL");
    }

    // DB-backed test: the summary counts must equal an independent hand-count of
    // the same eligible rows, including burst collapse.
    #[tokio::test]
    async fn summary_matches_collapsed_grid_counts() {
        use std::str::FromStr;
        // max_connections(1): a pool to `sqlite::memory:` otherwise gives each
        // connection its own empty DB, so migrations and inserts could land on
        // different databases. foreign_keys(false): we insert bare photo rows
        // without the full users/blobs graph, so don't enforce FK parents.
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        // Helper to insert a photo row with the columns the summary reads.
        async fn insert(
            pool: &sqlx::SqlitePool,
            id: &str,
            user: &str,
            media_type: &str,
            fav: bool,
            burst: Option<&str>,
        ) {
            sqlx::query(
                "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                 size_bytes, width, height, thumb_path, created_at, is_favorite, burst_id) \
                 VALUES (?, ?, ?, ?, ?, ?, 0, 0, 0, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(user)
            .bind(format!("{id}.jpg"))
            .bind(format!("uploads/{id}.jpg"))
            .bind("image/jpeg")
            .bind(media_type)
            .bind(format!(".thumbnails/{id}.jpg"))
            .bind("2026-01-01T00:00:00Z")
            .bind(fav)
            .bind(burst)
            .execute(pool)
            .await
            .unwrap();
        }

        let u = "user-1";
        // 3 loose photos (1 favorite), 1 gif, 1 video, plus a 3-frame burst.
        insert(&pool, "p1", u, "photo", true, None).await;
        insert(&pool, "p2", u, "photo", false, None).await;
        insert(&pool, "p3", u, "photo", false, None).await;
        insert(&pool, "g1", u, "gif", false, None).await;
        insert(&pool, "v1", u, "video", false, None).await;
        insert(&pool, "b1", u, "photo", false, Some("burst-A")).await;
        insert(&pool, "b2", u, "photo", false, Some("burst-A")).await;
        insert(&pool, "b3", u, "photo", false, Some("burst-A")).await;
        // A different user's row must not leak into the counts.
        insert(&pool, "x1", "user-2", "photo", true, None).await;

        let s = compute_summary(&pool, u).await.unwrap();
        assert_eq!(s.total, 8, "raw eligible rows for user-1");
        assert_eq!(s.photos, 6);
        assert_eq!(s.gifs, 1);
        assert_eq!(s.videos, 1);
        assert_eq!(s.audio, 0);
        assert_eq!(s.favorites, 1);
        // Grid: p1,p2,p3,g1,v1 (5 loose) + burst-A collapsed to 1 = 6 tiles.
        assert_eq!(s.collapsed_total, 6);
    }
}
