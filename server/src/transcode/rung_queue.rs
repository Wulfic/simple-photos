//! Which videos still owe the ladder a rung, and how many times we have tried.
//!
//! [`ladder`](super::ladder) decides which rungs a source *should* have and
//! [`renditions`](super::renditions) records the ones that exist. This module is
//! the queue between them: it selects work and it retires work that will not
//! succeed. It runs no ffmpeg and touches no files, which is what lets the
//! selection rules be tested against a real schema without a transcode.
//!
//! # The DB's geometry is a hint, not the encode's input
//!
//! Measured against the live library on 2026-07-21, `photos.width`/`height`
//! disagree with the ffprobe census in `todo.md` in two ways that matter:
//!
//! - **58 of 698 videos have no recorded geometry at all** (`width`/`height`
//!   ≤ 0). A prefilter that requires `min(w,h) > threshold` cannot see them, so
//!   selecting only on recorded geometry silently skips them forever.
//! - **Orientation disagrees.** The census reported 126 × `3840x2160`; the DB
//!   holds 78 × `2160x3840` plus 26 × `3840x2160`, and the same transposition
//!   shows up on `1440x1920` (census: `1920x1440`) and `4320x7680` (census:
//!   `7680x4320`).
//!
//! The second one is survivable *here* only because the ladder keys on
//! `min(width, height)`, which is orientation-independent — a transposed pair
//! yields the same short edge and therefore the same verdict. That is the
//! short-edge rule earning its keep a second time.
//!
//! It is **not** survivable downstream. [`ladder::rung_dimensions`] returns
//! `(width, height)` in the orientation it was given, and feeding a transposed
//! pair to `scale=W:H` squashes a landscape frame into a portrait box. So the
//! generation pass must take its dimensions from probing the file it is about to
//! encode, and must never take them from these columns. They are good enough to
//! *narrow* 698 videos to ~114, and good for nothing else.
//!
//! (Cause unconfirmed — most likely rotation side-data applied on one side and
//! not the other. It does not need to be resolved to build the queue, but it
//! does need to be resolved before the encode trusts any stored number.)

use crate::error::AppError;
use crate::gallery::eligibility::ELIGIBLE_PREDICATE;

use super::ladder::{rung_threshold, TIER_1080_SHORT_EDGE};

/// How many times a single rung may be attempted before it is retired.
///
/// Named rather than inlined, per `todo.md`'s standing instruction for the
/// conversion cap (#40). Three is the same budget that item chose: enough to
/// ride out a transient failure (a busy GPU, a full disk), few enough that a
/// genuinely broken file costs bounded work.
pub const MAX_RUNG_ATTEMPTS: i64 = 3;

/// A video that may still owe the ladder its 1080p rung.
///
/// "May" is load-bearing: for the 58 rows with no recorded geometry, candidacy
/// means "probe this to find out", not "this needs a rung".
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct RungCandidate {
    pub photo_id: String,
    pub user_id: String,
    pub filename: String,
    pub mime_type: String,
    /// Storage-root-relative path. Empty for rows that live only as a blob.
    pub file_path: String,
    /// Set in encrypted mode; the bytes must be decrypted before probing.
    pub encrypted_blob_id: Option<String>,
    pub width: i64,
    pub height: i64,
}

impl RungCandidate {
    /// Whether the recorded geometry can be believed enough to skip a probe.
    ///
    /// Even when true the generation pass still probes — see the module note on
    /// orientation. This distinguishes "narrowed by the prefilter" from
    /// "selected blind", which is only useful for logging and ordering.
    pub fn geometry_is_known(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

/// Videos that still owe the 1080p rung, cheapest first.
///
/// Cheapest-first rather than largest-first: the 4 live 8K sources are a
/// two-rung decision each and would otherwise sit at the head of the queue
/// blocking 110 smaller files. `conversion_priority` already sorts a mixed batch
/// fast-first for the same reason. Rows with unknown geometry sort ahead of
/// everything, because most of them will turn out to need no encode at all and
/// resolving them is one ffprobe.
///
/// `limit` bounds a single sweep so a boot on a large library has a predictable
/// cost; the next sweep picks up where this one stopped.
pub async fn find_rung_candidates(
    pool: &sqlx::SqlitePool,
    limit: i64,
) -> Result<Vec<RungCandidate>, AppError> {
    // The prefilter is deliberately WIDER than `ladder::needs_rung`. It must
    // admit unknown geometry (58 live rows) and let the probe decide, and it
    // must not admit anything `needs_rung` would reject on known geometry —
    // `prefilter_agrees_with_the_ladder_on_every_live_shape` pins both halves.
    //
    // Being wider is what makes `not_needed` (037) load-bearing rather than
    // cosmetic: a blind selection whose probe says "no rung owed" must be able
    // to record that as final, or the widening costs three ffprobes and a
    // spurious retirement warning per file, forever.
    let sql = format!(
        "SELECT p.id AS photo_id, p.user_id, p.filename, p.mime_type, \
                p.file_path, p.encrypted_blob_id, p.width, p.height \
         FROM photos p \
         WHERE p.media_type = 'video' \
           AND {ELIGIBLE_PREDICATE} \
           AND ( (p.width > 0 AND p.height > 0 AND MIN(p.width, p.height) > ?1) \
                 OR p.width <= 0 OR p.height <= 0 ) \
           AND NOT EXISTS ( \
                 SELECT 1 FROM video_renditions r \
                 WHERE r.photo_id = p.id AND r.short_edge = ?2 \
                   AND ( r.blob_id IS NOT NULL \
                         OR r.file_path IS NOT NULL \
                         OR r.not_needed = 1 \
                         OR r.attempt_count >= ?3 ) ) \
         ORDER BY CASE WHEN p.width <= 0 OR p.height <= 0 THEN 0 \
                       ELSE MIN(p.width, p.height) END ASC, \
                  p.id ASC \
         LIMIT ?4"
    );

    sqlx::query_as::<_, RungCandidate>(&sql)
        .bind(rung_threshold(TIER_1080_SHORT_EDGE))
        .bind(TIER_1080_SHORT_EDGE)
        .bind(MAX_RUNG_ATTEMPTS)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            tracing::error!("failed to select video rendition candidates: {e}");
            AppError::from(e)
        })
}

/// Charge an attempt against a rung and report the new count.
///
/// **Call this before the encode, not after it fails.** A file that OOMs the
/// process or hard-kills ffmpeg never reaches an error handler, and that is
/// exactly the file that must not be retried forever — `todo.md` names one
/// (`VIDEO0063.mp4`, no decodable video stream). Counting attempts rather than
/// failures is the only version of this cap that survives a crash.
///
/// The row it creates has neither locator, which `035` defines as "planned but
/// not produced" and `list_renditions` filters out, so a claim is never visible
/// to a picker. Width and height are left at 0 until an encode supplies real
/// ones; the ladder's geometry comes from the probe, never from here.
pub async fn begin_attempt(
    pool: &sqlx::SqlitePool,
    photo_id: &str,
    short_edge: i64,
) -> Result<i64, AppError> {
    sqlx::query(
        "INSERT INTO video_renditions \
           (photo_id, short_edge, width, height, is_source, size_bytes, \
            attempt_count, last_attempt_at, created_at) \
         VALUES (?, ?, 0, 0, 0, 0, 1, datetime('now'), datetime('now')) \
         ON CONFLICT(photo_id, short_edge) DO UPDATE SET \
           attempt_count   = attempt_count + 1, \
           last_attempt_at = datetime('now')",
    )
    .bind(photo_id)
    .bind(short_edge)
    .execute(pool)
    .await
    .map_err(|e| {
        tracing::error!(photo_id = %photo_id, short_edge, "failed to record rung attempt: {e}");
        AppError::from(e)
    })?;

    let count: i64 = sqlx::query_scalar(
        "SELECT attempt_count FROM video_renditions WHERE photo_id = ? AND short_edge = ?",
    )
    .bind(photo_id)
    .bind(short_edge)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!(photo_id = %photo_id, short_edge, "failed to read rung attempt count: {e}");
        AppError::from(e)
    })?;

    Ok(count)
}

/// Record that a rung is not owed at all, so the candidate query stops
/// returning this photo.
///
/// This is the verdict for a source the probe finds at or below the tier — the
/// expected outcome for most of the 58 rows selected blind because they have no
/// recorded geometry. It is a *success*: no encode was attempted and none was
/// needed, so the attempt budget is reset rather than spent. Leaving a charged
/// attempt behind would mean a file that is later replaced with a genuine 4K
/// source starts its real work with part of its retry budget already gone.
///
/// See `037_video_rendition_verdicts.sql` for why this cannot be expressed with
/// the locator columns alone. The row it writes stays unplayable, so no picker
/// can offer it and no client is nominated.
pub async fn mark_rung_not_needed(
    pool: &sqlx::SqlitePool,
    photo_id: &str,
    short_edge: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO video_renditions \
           (photo_id, short_edge, width, height, is_source, size_bytes, \
            not_needed, attempt_count, created_at) \
         VALUES (?, ?, 0, 0, 0, 0, 1, 0, datetime('now')) \
         ON CONFLICT(photo_id, short_edge) DO UPDATE SET \
           not_needed    = 1, \
           attempt_count = 0, \
           last_error    = NULL",
    )
    .bind(photo_id)
    .bind(short_edge)
    .execute(pool)
    .await
    .map_err(|e| {
        tracing::error!(
            photo_id = %photo_id,
            short_edge,
            "failed to record that no rung is owed: {e}"
        );
        AppError::from(e)
    })?;
    Ok(())
}

/// Attach the reason a rung attempt failed.
///
/// Separate from [`begin_attempt`] because the count must already be committed
/// by the time the encode starts. This only annotates it, so a failure that
/// never returns simply leaves `last_error` NULL against a charged attempt —
/// which reads correctly as "we tried and never came back".
///
/// Every failure path logs: a rung that vanishes without a trace is a 4K encode
/// of wasted CPU with nothing to attribute it to. Retirement is logged at
/// `warn`, because a file the user will never get a picker for is a thing an
/// operator should be able to find (#45 will surface it in the audit log).
pub async fn record_failure(
    pool: &sqlx::SqlitePool,
    photo_id: &str,
    short_edge: i64,
    attempt: i64,
    error: &str,
) -> Result<(), AppError> {
    // Truncate: ffmpeg failures can carry kilobytes of filter-graph dump, and
    // this column is read by an operator, not a parser.
    let truncated: String = error.chars().take(500).collect();

    sqlx::query("UPDATE video_renditions SET last_error = ? WHERE photo_id = ? AND short_edge = ?")
        .bind(&truncated)
        .bind(photo_id)
        .bind(short_edge)
        .execute(pool)
        .await
        .map_err(|e| {
            tracing::error!(photo_id = %photo_id, short_edge, "failed to record rung failure: {e}");
            AppError::from(e)
        })?;

    if attempt >= MAX_RUNG_ATTEMPTS {
        tracing::warn!(
            photo_id = %photo_id,
            short_edge,
            attempt,
            "retiring video rendition after {MAX_RUNG_ATTEMPTS} attempts; this video \
             will not get a quality picker: {truncated}"
        );
    } else {
        tracing::info!(
            photo_id = %photo_id,
            short_edge,
            attempt,
            "video rendition attempt failed, will retry: {truncated}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcode::ladder::{needs_rung, short_edge};
    use crate::transcode::renditions::{list_renditions, upsert_rendition, StoredRendition};
    use std::str::FromStr;

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

    async fn insert_video(pool: &sqlx::SqlitePool, id: &str, w: i64, h: i64) {
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, \
             media_type, size_bytes, width, height, created_at) \
             VALUES (?, 'u1', ?, ?, 'video/mp4', 'video', 0, ?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("{id}.mp4"))
        .bind(format!("uploads/{id}.mp4"))
        .bind(w)
        .bind(h)
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

    async fn candidate_ids(pool: &sqlx::SqlitePool) -> Vec<String> {
        find_rung_candidates(pool, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.photo_id)
            .collect()
    }

    async fn head_seq(pool: &sqlx::SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM photo_change_log")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn produced(photo_id: &str, blob: &str) -> StoredRendition {
        StoredRendition {
            photo_id: photo_id.into(),
            short_edge: TIER_1080_SHORT_EDGE,
            width: 1920,
            height: 1080,
            is_source: 0,
            blob_id: Some(blob.into()),
            file_path: None,
            codec: Some("h264".into()),
            bitrate: Some(4_000_000),
            size_bytes: 2048,
        }
    }

    /// The whole point of the queue: a source above the tier is work, a source
    /// at or below it is not, and the two traps from the live library stay out.
    #[tokio::test]
    async fn only_oversized_videos_are_candidates() {
        let pool = test_pool().await;
        insert_video(&pool, "p_4k", 3840, 2160).await;
        insert_video(&pool, "p_1080", 1920, 1080).await;
        insert_video(&pool, "p_portrait", 1080, 1920).await; // IS the 1080p tier
        insert_video(&pool, "p_padded", 2288, 1088).await; // macroblock padding
        insert_video(&pool, "p_720", 1280, 720).await;

        assert_eq!(candidate_ids(&pool).await, vec!["p_4k"]);
    }

    /// Stills must never enter a video ladder, however large they are.
    #[tokio::test]
    async fn non_video_media_is_never_a_candidate() {
        let pool = test_pool().await;
        insert_video(&pool, "p_vid", 3840, 2160).await;
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, \
             media_type, size_bytes, width, height, created_at) \
             VALUES ('p_img', 'u1', 'a.jpg', 'uploads/a.jpg', 'image/jpeg', \
                     'photo', 0, 6000, 4000, '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(candidate_ids(&pool).await, vec!["p_vid"]);
    }

    /// 58 live videos have no recorded geometry. Requiring `min(w,h) > 1188`
    /// makes them permanently invisible, so the prefilter must admit them and
    /// let the probe decide.
    #[tokio::test]
    async fn videos_with_unknown_geometry_are_selected_for_probing() {
        let pool = test_pool().await;
        insert_video(&pool, "p_unknown", 0, 0).await;
        insert_video(&pool, "p_half", 1920, 0).await;
        insert_video(&pool, "p_small", 640, 480).await;

        let found = find_rung_candidates(&pool, 100).await.unwrap();
        let ids: Vec<&str> = found.iter().map(|c| c.photo_id.as_str()).collect();
        assert!(ids.contains(&"p_unknown"));
        assert!(
            ids.contains(&"p_half"),
            "one usable edge is still unknown geometry"
        );
        assert!(
            !ids.contains(&"p_small"),
            "known-and-small must not be dragged in with the unknowns"
        );

        // The generation pass branches on this to decide whether the prefilter
        // narrowed the row or selected it blind. Both of these were selected
        // blind and must say so.
        for c in &found {
            assert!(
                !c.geometry_is_known(),
                "{} was selected without usable geometry and must report it",
                c.photo_id
            );
        }
    }

    /// Unknown geometry is one ffprobe; an 8K source is a two-rung re-encode.
    /// Ordering by cost keeps the 4 live 8K files from heading the queue.
    #[tokio::test]
    async fn candidates_come_back_cheapest_first() {
        let pool = test_pool().await;
        insert_video(&pool, "p_8k", 7680, 4320).await;
        insert_video(&pool, "p_4k", 3840, 2160).await;
        insert_video(&pool, "p_1440", 1920, 1440).await;
        insert_video(&pool, "p_unknown", 0, 0).await;

        assert_eq!(
            candidate_ids(&pool).await,
            vec!["p_unknown", "p_1440", "p_4k", "p_8k"]
        );
    }

    /// Success is what makes the queue self-limiting.
    #[tokio::test]
    async fn a_produced_rung_removes_its_photo_from_the_queue() {
        let pool = test_pool().await;
        insert_video(&pool, "p_4k", 3840, 2160).await;
        insert_blob(&pool, "b1").await;
        assert_eq!(candidate_ids(&pool).await, vec!["p_4k"]);

        upsert_rendition(&pool, &produced("p_4k", "b1"))
            .await
            .unwrap();

        assert!(
            candidate_ids(&pool).await.is_empty(),
            "a produced rung must not be re-encoded on the next sweep"
        );
    }

    /// The failure loop this migration exists to prevent. Without the cap a 4K
    /// re-encode is retried on every sweep, forever.
    #[tokio::test]
    async fn a_rung_is_retired_after_three_attempts() {
        let pool = test_pool().await;
        insert_video(&pool, "p_4k", 3840, 2160).await;

        for expected in 1..=MAX_RUNG_ATTEMPTS {
            assert_eq!(
                candidate_ids(&pool).await,
                vec!["p_4k"],
                "attempt {expected} should still be offered"
            );
            let n = begin_attempt(&pool, "p_4k", TIER_1080_SHORT_EDGE)
                .await
                .unwrap();
            assert_eq!(n, expected);
            record_failure(&pool, "p_4k", TIER_1080_SHORT_EDGE, n, "ffmpeg exploded")
                .await
                .unwrap();
        }

        assert!(
            candidate_ids(&pool).await.is_empty(),
            "after {MAX_RUNG_ATTEMPTS} attempts the video must be retired, not retried forever"
        );
        let err: Option<String> =
            sqlx::query_scalar("SELECT last_error FROM video_renditions WHERE photo_id = 'p_4k'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(err.as_deref(), Some("ffmpeg exploded"));
    }

    /// The cap must hold even when the failure never returns — a process kill
    /// mid-encode is exactly the case that would otherwise loop forever.
    #[tokio::test]
    async fn attempts_are_charged_before_the_encode_so_a_crash_still_counts() {
        let pool = test_pool().await;
        insert_video(&pool, "p_4k", 3840, 2160).await;

        // Three claims, no `record_failure` at all: the encode never came back.
        for _ in 0..MAX_RUNG_ATTEMPTS {
            begin_attempt(&pool, "p_4k", TIER_1080_SHORT_EDGE)
                .await
                .unwrap();
        }

        assert!(
            candidate_ids(&pool).await.is_empty(),
            "an encode that kills the process must still consume its attempt"
        );
    }

    /// A claim is not a rendition. `035` defines the no-locator state as
    /// "planned but not produced", and a picker offering it would 404.
    #[tokio::test]
    async fn a_claimed_attempt_is_never_offered_to_a_picker() {
        let pool = test_pool().await;
        insert_video(&pool, "p_4k", 3840, 2160).await;

        begin_attempt(&pool, "p_4k", TIER_1080_SHORT_EDGE)
            .await
            .unwrap();

        assert!(
            list_renditions(&pool, "p_4k").await.unwrap().is_empty(),
            "an in-flight attempt must not reach the picker"
        );
    }

    /// Verified against SQLite, not assumed: an upsert taking the DO UPDATE
    /// branch fires UPDATE triggers, so `035`'s INSERT-only nomination never
    /// fires on the claim-then-fill path. Without the UPDATE trigger added in
    /// `036` the rung becomes playable and no client is ever told.
    #[tokio::test]
    async fn filling_in_a_claimed_rung_nominates_the_photo_for_re_sync() {
        let pool = test_pool().await;
        insert_video(&pool, "p_4k", 3840, 2160).await;
        insert_blob(&pool, "b1").await;

        begin_attempt(&pool, "p_4k", TIER_1080_SHORT_EDGE)
            .await
            .unwrap();
        let before = head_seq(&pool).await;

        upsert_rendition(&pool, &produced("p_4k", "b1"))
            .await
            .unwrap();

        assert!(
            head_seq(&pool).await > before,
            "the moment a rung becomes playable is an UPDATE, not an INSERT — \
             without an AFTER UPDATE trigger the picker stays empty forever"
        );
    }

    /// The other half of "nominate exactly when playability changes": a failed
    /// attempt has nothing to tell a client, and waking every client once per
    /// attempt is pure cost.
    #[tokio::test]
    async fn a_failed_attempt_does_not_nominate_the_photo() {
        let pool = test_pool().await;
        insert_video(&pool, "p_4k", 3840, 2160).await;
        let before = head_seq(&pool).await;

        let n = begin_attempt(&pool, "p_4k", TIER_1080_SHORT_EDGE)
            .await
            .unwrap();
        record_failure(&pool, "p_4k", TIER_1080_SHORT_EDGE, n, "nope")
            .await
            .unwrap();
        begin_attempt(&pool, "p_4k", TIER_1080_SHORT_EDGE)
            .await
            .unwrap();

        assert_eq!(
            head_seq(&pool).await,
            before,
            "claims and failures are invisible to clients and must not move head_seq"
        );
    }

    /// A video hidden in a secure gallery is not in the feed, so spending a
    /// 4K re-encode on it buys nothing. Same predicate as the gallery, by
    /// construction rather than by copy.
    #[tokio::test]
    async fn secure_gallery_videos_are_not_candidates() {
        let pool = test_pool().await;
        insert_video(&pool, "p_open", 3840, 2160).await;
        insert_video(&pool, "p_secure", 3840, 2160).await;
        // The first eligibility arm matches a photo id against `blob_id`, which
        // carries a FK to `blobs` — so a secured photo's id is also a blob id.
        insert_blob(&pool, "p_secure").await;
        sqlx::query(
            "INSERT INTO encrypted_galleries (id, user_id, name, password_hash, created_at) \
             VALUES ('g1', 'u1', 'vault', 'x', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at) \
             VALUES ('i1', 'g1', 'p_secure', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(candidate_ids(&pool).await, vec!["p_open"]);
    }

    /// A sweep must have a predictable cost on a large library.
    #[tokio::test]
    async fn the_sweep_is_bounded_by_its_limit() {
        let pool = test_pool().await;
        for i in 0..10 {
            insert_video(&pool, &format!("p{i:02}"), 3840, 2160).await;
        }

        assert_eq!(find_rung_candidates(&pool, 3).await.unwrap().len(), 3);
    }

    /// The prefilter is SQL and `needs_rung` is Rust, and they must not drift.
    /// Runs the live library's measured shapes through both: on known geometry
    /// the verdicts must be identical, which is the property that keeps the
    /// queue from either skipping work or booking 4K encodes it should not.
    #[tokio::test]
    async fn prefilter_agrees_with_the_ladder_on_every_live_shape() {
        // Shapes as the DB actually holds them (2026-07-21), including the
        // transposed orientations the ffprobe census did not show.
        const LIVE_SHAPES: &[(i64, i64)] = &[
            (2160, 3840),
            (3840, 2160),
            (1440, 1920),
            (4320, 7680),
            (2288, 1088),
            (1080, 1920),
            (1920, 1080),
            (2400, 1080),
            (768, 1024),
            (1280, 720),
            (640, 480),
        ];

        let pool = test_pool().await;
        for (i, (w, h)) in LIVE_SHAPES.iter().enumerate() {
            insert_video(&pool, &format!("p{i:02}"), *w, *h).await;
        }

        let selected = candidate_ids(&pool).await;
        for (i, (w, h)) in LIVE_SHAPES.iter().enumerate() {
            let id = format!("p{i:02}");
            assert_eq!(
                selected.contains(&id),
                needs_rung(*w, *h, TIER_1080_SHORT_EDGE),
                "SQL prefilter and ladder::needs_rung disagree about {w}x{h} \
                 (short edge {})",
                short_edge(*w, *h),
            );
        }
    }

    /// Orientation must not change the verdict. This is why the transposition
    /// measured between the DB and the ffprobe census is survivable at
    /// selection time — and it is the only place it is survivable.
    #[tokio::test]
    async fn a_transposed_pair_yields_the_same_verdict() {
        let pool = test_pool().await;
        insert_video(&pool, "p_land", 3840, 2160).await;
        insert_video(&pool, "p_port", 2160, 3840).await;
        insert_video(&pool, "p_land_small", 1920, 1080).await;
        insert_video(&pool, "p_port_small", 1080, 1920).await;

        let ids = candidate_ids(&pool).await;
        assert!(ids.contains(&"p_land".to_string()));
        assert!(ids.contains(&"p_port".to_string()));
        assert!(!ids.contains(&"p_land_small".to_string()));
        assert!(!ids.contains(&"p_port_small".to_string()));
    }
}
