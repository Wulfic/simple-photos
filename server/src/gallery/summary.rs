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

/// Number of tiles the "Recently Added" smart album caps at. MUST match
/// `SMART_ALBUM_DEFS["smart-recent"].limit` in `web/src/gallery/smartAlbums.ts`
/// — if these drift, the badge lies about the grid again.
pub const RECENT_ALBUM_LIMIT: i64 = 100;

/// Precomputed gallery counts for a single user. All counts reflect the same
/// rows `encrypted-sync` returns (secure-gallery items excluded).
///
/// Two families of number live here and they are NOT interchangeable:
///
/// * The `total`/`photos`/`gifs`/`videos`/`audio`/`favorites` block is **raw
///   media-type row counts** — one per database row, no burst collapse. Kept
///   for existing consumers and for diagnostics.
/// * The `smart_*` block is **tile counts**: exactly what the corresponding
///   smart-album grid renders, i.e. the client's filter applied first and burst
///   frames collapsed second, in that order. Badges must use these.
///
/// The distinction is the single most common source of count bugs in this repo
/// (#42 and predecessors), so it is spelled out rather than implied. Note in
/// particular that `photos` counts `media_type = 'photo'` only, while
/// `smart_photos` counts photos AND gifs, because the client's "Photos" smart
/// album is defined that way.
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

    /// Tiles in the "Photos" smart album (photo + gif, burst-collapsed).
    pub smart_photos: i64,
    /// Tiles in the "GIFs" smart album.
    pub smart_gifs: i64,
    /// Tiles in the "Videos" smart album.
    pub smart_videos: i64,
    /// Tiles in the "Audio" smart album.
    pub smart_audio: i64,
    /// Tiles in the "Favorites" smart album.
    pub smart_favorites: i64,
    /// Tiles in "Recently Added" — the whole library, capped at
    /// [`RECENT_ALBUM_LIMIT`].
    pub smart_recent: i64,

    /// Head of the change log (#38). Two jobs, both cheap:
    ///
    /// * **Change detection.** A client that already holds this sequence knows
    ///   nothing has changed and can skip `encrypted-sync` altogether — no
    ///   pagination, no IndexedDB writes, no blob downloads. That is the
    ///   steady-state property #38 is about, and it costs one `MAX(seq)` here
    ///   instead of a full-library walk there.
    /// * **Integrity backstop.** Delta sync's residual risk is a write path
    ///   that bypasses triggers entirely (a wholesale DB restore, say). A
    ///   client comparing its mirror against `total` can detect that drift and
    ///   fall back to a full walk, so a missed change degrades to "stale until
    ///   the next check" rather than "silently wrong forever".
    ///
    /// Never served from the TTL cache — see [`photos_summary`].
    pub head_seq: i64,
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
    // burst_id)` ignores NULLs in SQLite, so it counts only real burst groups —
    // and `COUNT(DISTINCT CASE WHEN <pred> THEN burst_id END)` narrows that to
    // the groups a given smart-album filter matches, which is what makes
    // filter-then-collapse expressible in a single aggregate.
    let r: SummaryRow = sqlx::query_as(&format!(
        "SELECT \
           COUNT(*) AS total, \
           COALESCE(SUM(CASE WHEN media_type = 'photo' THEN 1 ELSE 0 END), 0) AS photos, \
           COALESCE(SUM(CASE WHEN media_type = 'gif'   THEN 1 ELSE 0 END), 0) AS gifs, \
           COALESCE(SUM(CASE WHEN media_type = 'video' THEN 1 ELSE 0 END), 0) AS videos, \
           COALESCE(SUM(CASE WHEN media_type = 'audio' THEN 1 ELSE 0 END), 0) AS audio, \
           COALESCE(SUM(CASE WHEN is_favorite THEN 1 ELSE 0 END), 0) AS favorites, \
           COALESCE(SUM(CASE WHEN burst_id IS NULL THEN 1 ELSE 0 END), 0) AS non_burst, \
           COUNT(DISTINCT burst_id) AS burst_groups, \
           COALESCE(SUM(CASE WHEN media_type IN ('photo','gif') AND burst_id IS NULL THEN 1 ELSE 0 END), 0) AS photos_nb, \
           COUNT(DISTINCT CASE WHEN media_type IN ('photo','gif') THEN burst_id END) AS photos_bg, \
           COALESCE(SUM(CASE WHEN media_type = 'gif' AND burst_id IS NULL THEN 1 ELSE 0 END), 0) AS gifs_nb, \
           COUNT(DISTINCT CASE WHEN media_type = 'gif' THEN burst_id END) AS gifs_bg, \
           COALESCE(SUM(CASE WHEN media_type = 'video' AND burst_id IS NULL THEN 1 ELSE 0 END), 0) AS videos_nb, \
           COUNT(DISTINCT CASE WHEN media_type = 'video' THEN burst_id END) AS videos_bg, \
           COALESCE(SUM(CASE WHEN media_type = 'audio' AND burst_id IS NULL THEN 1 ELSE 0 END), 0) AS audio_nb, \
           COUNT(DISTINCT CASE WHEN media_type = 'audio' THEN burst_id END) AS audio_bg, \
           COALESCE(SUM(CASE WHEN is_favorite AND burst_id IS NULL THEN 1 ELSE 0 END), 0) AS favorites_nb, \
           COUNT(DISTINCT CASE WHEN is_favorite THEN burst_id END) AS favorites_bg \
         FROM photos p \
         WHERE {eligible}",
        eligible = crate::gallery::eligibility::eligible_for_user()
    ))
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    let collapsed = collapsed_total(r.non_burst, r.burst_groups);
    Ok(PhotoSummary {
        total: r.total,
        collapsed_total: collapsed,
        photos: r.photos,
        gifs: r.gifs,
        videos: r.videos,
        audio: r.audio,
        favorites: r.favorites,
        smart_photos: collapsed_total(r.photos_nb, r.photos_bg),
        smart_gifs: collapsed_total(r.gifs_nb, r.gifs_bg),
        smart_videos: collapsed_total(r.videos_nb, r.videos_bg),
        smart_audio: collapsed_total(r.audio_nb, r.audio_bg),
        smart_favorites: collapsed_total(r.favorites_nb, r.favorites_bg),
        smart_recent: collapsed.min(RECENT_ALBUM_LIMIT),
        head_seq: crate::gallery::sync::head_seq(pool).await?,
    })
}

/// Raw aggregate row backing [`compute_summary`]. Named fields rather than a
/// wide tuple so the eighteen columns cannot be silently transposed.
#[derive(sqlx::FromRow)]
struct SummaryRow {
    total: i64,
    photos: i64,
    gifs: i64,
    videos: i64,
    audio: i64,
    favorites: i64,
    non_burst: i64,
    burst_groups: i64,
    photos_nb: i64,
    photos_bg: i64,
    gifs_nb: i64,
    gifs_bg: i64,
    videos_nb: i64,
    videos_bg: i64,
    audio_nb: i64,
    audio_bg: i64,
    favorites_nb: i64,
    favorites_bg: i64,
}

/// GET /api/photos/summary
///
/// Cheap, cached, one-round-trip counts for smart-album badges. Clients render
/// counts from this instantly, before/without any full gallery sync.
pub async fn photos_summary(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<PhotoSummary>, AppError> {
    // Counts tolerate the TTL; `head_seq` must not. Clients use it to decide
    // whether to sync at all, so serving a cached (stale, lower) head would
    // make them re-fetch changes they already have on every poll for up to the
    // TTL — the exact busywork #38 is removing. One indexed MAX(seq) is far
    // cheaper than the aggregate the cache is actually protecting.
    if let Some(mut cached) = state.summary_cache.get_fresh(&auth.user_id) {
        cached.head_seq = crate::gallery::sync::head_seq(&state.read_pool).await?;
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
        assert!(
            cache.get_fresh("u1").is_none(),
            "entry should expire after TTL"
        );
    }

    // max_connections(1): a pool to `sqlite::memory:` otherwise gives each
    // connection its own empty DB, so migrations and inserts could land on
    // different databases. foreign_keys(false): we insert bare photo rows
    // without the full users/blobs graph, so don't enforce FK parents.
    async fn test_summary_pool() -> sqlx::SqlitePool {
        use std::str::FromStr;
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

    /// Insert a photo row with the columns the summary reads.
    async fn insert_summary_row(
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

    // DB-backed test: the summary counts must equal an independent hand-count of
    // the same eligible rows, including burst collapse.
    #[tokio::test]
    async fn summary_matches_collapsed_grid_counts() {
        let pool = test_summary_pool().await;
        let insert = insert_summary_row;

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

        // Smart-album TILE counts. "Photos" is photo+gif per the client's
        // SMART_ALBUM_DEFS, so: p1,p2,p3 + g1 loose = 4, plus burst-A = 5.
        assert_eq!(s.smart_photos, 5);
        assert_eq!(s.smart_gifs, 1);
        assert_eq!(s.smart_videos, 1);
        assert_eq!(s.smart_audio, 0);
        assert_eq!(s.smart_favorites, 1);
        assert_eq!(s.smart_recent, 6, "under the cap, equals collapsed_total");
    }

    /// The ordering trap: the client filters and THEN collapses. A burst with
    /// several favourited frames is ONE favourite tile, not several. Collapsing
    /// first (or counting raw rows) is how the badges drifted from the grid.
    #[tokio::test]
    async fn smart_counts_filter_before_collapsing() {
        let pool = test_summary_pool().await;
        let u = "user-1";

        // A 3-frame burst with 2 frames favourited, plus one loose favourite.
        insert_summary_row(&pool, "b1", u, "photo", true, Some("burst-A")).await;
        insert_summary_row(&pool, "b2", u, "photo", true, Some("burst-A")).await;
        insert_summary_row(&pool, "b3", u, "photo", false, Some("burst-A")).await;
        insert_summary_row(&pool, "loose", u, "photo", true, None).await;

        let s = compute_summary(&pool, u).await.unwrap();

        // Raw favourites still counts rows: b1, b2, loose.
        assert_eq!(s.favorites, 3, "raw favourite ROWS");
        // But the Favorites grid renders 2 tiles: the burst + the loose photo.
        assert_eq!(
            s.smart_favorites, 2,
            "favourited burst frames collapse to ONE tile"
        );
        assert_eq!(s.collapsed_total, 2, "burst-A + loose");
        assert_eq!(s.smart_photos, 2);
    }

    /// "Recently Added" is capped client-side; the badge must respect the cap
    /// or it promises more tiles than the grid will ever render.
    #[tokio::test]
    async fn recent_album_count_is_capped() {
        let pool = test_summary_pool().await;
        let u = "user-1";
        for i in 0..(RECENT_ALBUM_LIMIT + 25) {
            insert_summary_row(&pool, &format!("r{i:04}"), u, "photo", false, None).await;
        }

        let s = compute_summary(&pool, u).await.unwrap();
        assert_eq!(s.total, RECENT_ALBUM_LIMIT + 25);
        assert_eq!(s.smart_recent, RECENT_ALBUM_LIMIT, "capped at the limit");
    }
}
