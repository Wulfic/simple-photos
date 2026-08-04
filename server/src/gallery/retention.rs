//! Change-log retention for the delta sync feed (#38).
//!
//! ## What actually grows, and what does not
//!
//! `photo_change_log.photo_id` is the PRIMARY KEY — one row per photo, its
//! `seq` bumped in place. So the log does **not** grow per change; a photo
//! edited a thousand times still occupies one row, and for live photos the log
//! is bounded by the library size. That much was designed in from migration
//! `033` and needs no help here.
//!
//! What is genuinely unbounded is the **tombstone**: when a photo row is
//! deleted its change-log row deliberately survives (no FK, by design — a
//! tombstone whose evidence is cascaded away is not a tombstone). Nothing has
//! ever removed those, so the log's floor is "every photo id this server has
//! ever seen", which only ever rises.
//!
//! ## Why a bare `DELETE` would be a data-loss bug
//!
//! A tombstone is the *only* record that a photo left the feed. A client at
//! cursor `S` is promised every change with `seq > S`; delete a tombstone at
//! `seq > S` and that client keeps the row forever, because no future response
//! will ever mention that id again. The full walk is self-healing, the delta
//! feed is not — that asymmetry is the whole of #38's risk.
//!
//! So pruning is a **policy**, not a statement:
//!
//! 1. Prune only rows whose photo is genuinely gone, older than the window.
//! 2. Record how far the prune reached, as [`pruned_through_seq`].
//! 3. Refuse to serve a delta below that floor — hand back the **full walk**
//!    instead, which is self-healing and repairs whatever the client missed.
//!
//! Step 3 costs nothing in client code: both clients already treat an absent
//! `deleted` array as "this server did not honour `since`, restart as a full
//! walk" (the pre-#38-server handshake). A beyond-retention client and an
//! old-server client want the identical recovery, so they share the branch.
//!
//! ## Two traps, both pinned by tests below
//!
//! - **Compute the cutoff in SQLite, not in Rust.** `changed_at` is written by
//!   the `033` triggers as `datetime('now')` — `"2026-08-03 12:00:00"`. The
//!   obvious Rust equivalent, `chrono::Utc::now().to_rfc3339()`, formats as
//!   `"2026-08-03T12:00:00.123456+00:00"`, and these are compared as **strings**
//!   (SQLite has no date type). On rows dated the same day as the cutoff the
//!   comparison then turns on `' '` (0x20) vs `'T'` (0x54), so a row that is
//!   *inside* the window reads as older than it and is pruned up to a day
//!   early — plus whatever the fractional seconds and `+00:00` suffix add. Not
//!   catastrophic on its own, but the boundary of a retention policy is the
//!   only part of it anyone can observe. The cutoff is therefore
//!   `datetime('now', '-N days')`, produced by the same function that wrote the
//!   column. `a_tombstone_just_inside_the_window_survives` is built on exactly
//!   that same-day boundary and fails if this is ever "tidied" into chrono.
//! - **Never let the head go backwards.** Every trigger computes
//!   `MAX(seq) + 1`, so deleting the highest-seq row makes the next change
//!   reuse a sequence a client has already passed — that change becomes
//!   invisible to it forever. The victim predicate excludes the current
//!   maximum unconditionally, age be damned; it is one row.

use crate::error::AppError;

/// How long a tombstone stays readable before a client that has not synced in
/// that long is pushed onto the full walk.
///
/// Deliberately generous, because the trade is lopsided: keeping a tombstone
/// costs **one row**, while dropping one too early costs every stale client a
/// complete library re-walk. Phones are offline for weeks at a time. 90 days
/// is long enough that the fallback is a genuine edge case and short enough
/// that a library with heavy delete churn still converges.
pub const TOMBSTONE_RETENTION_DAYS: i64 = 90;

/// `server_settings` key holding the highest sequence any prune has removed.
const PRUNED_THROUGH_KEY: &str = "photo_change_log_pruned_through";

/// Which rows a prune may take. Interpolated into both the "how far did we
/// reach" probe and the `DELETE` itself so the two cannot drift — the repo has
/// six recorded instances of one list copied into two places and diverging.
///
/// `?1` is the SQLite date modifier (`"-90 days"`).
///
/// The three arms, in the order they matter:
/// - `NOT EXISTS (... photos ...)` — a tombstone proper. A **secure-hidden**
///   photo still has its `photos` row, so it is spared: its change-log entry is
///   the only thing telling clients to drop a photo the user just secured, and
///   pruning it would leave that photo visible on every stale client.
/// - `seq < (SELECT MAX(seq) ...)` — head monotonicity, see the module doc.
/// - `changed_at < datetime('now', ?1)` — the window, computed by SQLite.
const VICTIM_PREDICATE: &str = "NOT EXISTS (SELECT 1 FROM photos \
                                WHERE photos.id = photo_change_log.photo_id) \
     AND seq < (SELECT MAX(seq) FROM photo_change_log) \
     AND changed_at < datetime('now', ?1)";

/// What one prune pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PruneOutcome {
    /// Tombstones removed.
    pub pruned: u64,
    /// The retention floor after this pass — see [`pruned_through_seq`].
    pub floor: i64,
}

/// The highest change-log sequence that has ever been pruned away.
///
/// A delta request with `since` **below** this cannot be answered correctly:
/// some tombstone in `(since, floor]` no longer exists, so the response would
/// omit a removal the client still holds. `gallery::sync::fetch_delta` reads
/// this on every delta request and falls back to the full walk when it must.
///
/// Zero on a server that has never pruned, which makes every cursor valid —
/// the pre-retention behaviour exactly.
pub async fn pruned_through_seq(pool: &sqlx::SqlitePool) -> Result<i64, AppError> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = ?")
            .bind(PRUNED_THROUGH_KEY)
            .fetch_optional(pool)
            .await?;

    // An unparseable value must not be read as "no floor" — that would serve
    // deltas the prune has already invalidated. Treat corruption as the most
    // conservative floor we can name and log it, because it is unreachable
    // unless something outside this module wrote the key.
    Ok(match raw {
        None => 0,
        Some(v) => v.trim().parse::<i64>().unwrap_or_else(|_| {
            tracing::error!(
                key = PRUNED_THROUGH_KEY, value = %v,
                "retention floor is not an integer; forcing every client onto the full walk"
            );
            i64::MAX
        }),
    })
}

/// Drop tombstones older than `retention_days` and raise the retention floor.
///
/// One transaction: the floor and the deletion must land together, or a crash
/// between them leaves rows gone with no record that they ever existed —
/// precisely the silent-ghost-row failure this module exists to prevent.
pub async fn prune_change_log(
    pool: &sqlx::SqlitePool,
    retention_days: i64,
) -> Result<PruneOutcome, AppError> {
    let modifier = format!("-{retention_days} days");
    let mut tx = pool.begin().await?;

    // How far this prune reaches, measured BEFORE the delete and with the
    // identical predicate. Zero when there is nothing to take.
    let reach: i64 = sqlx::query_scalar(&format!(
        "SELECT COALESCE(MAX(seq), 0) FROM photo_change_log WHERE {VICTIM_PREDICATE}"
    ))
    .bind(&modifier)
    .fetch_one(&mut *tx)
    .await?;

    let previous: i64 = sqlx::query_scalar("SELECT value FROM server_settings WHERE key = ?")
        .bind(PRUNED_THROUGH_KEY)
        .fetch_optional(&mut *tx)
        .await?
        .and_then(|v: String| v.trim().parse::<i64>().ok())
        .unwrap_or(0);

    if reach == 0 {
        // Nothing eligible. Commit nothing and leave the floor where it is —
        // an early return keeps a no-op hourly pass from writing to
        // server_settings 24 times a day forever.
        tx.rollback().await?;
        return Ok(PruneOutcome { pruned: 0, floor: previous });
    }

    let deleted = sqlx::query(&format!(
        "DELETE FROM photo_change_log WHERE {VICTIM_PREDICATE}"
    ))
    .bind(&modifier)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    // `max` rather than a bare assignment. Sequence and timestamp are written
    // together by the triggers, so an age-ordered prune is also seq-ordered and
    // this should never bind — but a floor that can move DOWN silently
    // re-validates cursors this module has already invalidated, and that is not
    // a failure mode worth leaving to an argument about ordering.
    let floor = previous.max(reach);

    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(PRUNED_THROUGH_KEY)
    .bind(floor.to_string())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    tracing::info!(
        pruned = deleted, floor,
        "pruned change-log tombstones; clients below the floor will full-walk"
    );

    Ok(PruneOutcome { pruned: deleted, floor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // See `summary.rs` for why max_connections(1) + foreign_keys(false).
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

    async fn insert(pool: &sqlx::SqlitePool, id: &str, user: &str) {
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, thumb_path, created_at, taken_at, is_favorite) \
             VALUES (?, ?, ?, ?, 'image/jpeg', 'photo', 0, 0, 0, ?, ?, ?, 0)",
        )
        .bind(id)
        .bind(user)
        .bind(format!("{id}.jpg"))
        .bind(format!("uploads/{id}.jpg"))
        .bind(format!(".thumbnails/{id}.jpg"))
        .bind("2026-01-01T00:00:00Z")
        .bind("2026-01-01T00:00:00Z")
        .execute(pool)
        .await
        .unwrap();
    }

    /// Push a log row's timestamp into the past. Real ageing is not available
    /// in a unit test, and a `retention_days = 0` shortcut would be flaky:
    /// `datetime('now')` has one-second resolution, so a row written in the
    /// same second is not `< datetime('now')`.
    async fn backdate(pool: &sqlx::SqlitePool, photo_id: &str, days: i64) {
        sqlx::query(&format!(
            "UPDATE photo_change_log SET changed_at = datetime('now', '-{days} days') \
             WHERE photo_id = ?"
        ))
        .bind(photo_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Move the head past whatever just happened, by making a later change.
    ///
    /// Needed in almost every test here, and it is not test scaffolding — it
    /// is the behaviour: a deletion is by definition the most recent event at
    /// the instant it occurs, so its tombstone IS the head until something
    /// else changes. Without this, a test "spared by the date window" would
    /// really be spared by the head guard and would pass with the window
    /// deleted entirely.
    async fn bump_head(pool: &sqlx::SqlitePool, marker: &str) {
        insert(pool, marker, "user-1").await;
    }

    async fn log_ids(pool: &sqlx::SqlitePool) -> Vec<String> {
        sqlx::query_scalar("SELECT photo_id FROM photo_change_log ORDER BY photo_id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn head(pool: &sqlx::SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM photo_change_log")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// The baseline: an aged tombstone is what this module is for.
    #[tokio::test]
    async fn an_aged_tombstone_is_pruned_and_raises_the_floor() {
        let pool = test_pool().await;
        insert(&pool, "keep", "user-1").await;
        insert(&pool, "gone", "user-1").await;
        insert(&pool, "recent", "user-1").await;

        sqlx::query("DELETE FROM photos WHERE id = 'gone'")
            .execute(&pool)
            .await
            .unwrap();
        let gone_seq: i64 =
            sqlx::query_scalar("SELECT seq FROM photo_change_log WHERE photo_id = 'gone'")
                .fetch_one(&pool)
                .await
                .unwrap();
        backdate(&pool, "gone", 100).await;
        bump_head(&pool, "later").await;

        assert_eq!(pruned_through_seq(&pool).await.unwrap(), 0, "no prune yet");

        let outcome = prune_change_log(&pool, TOMBSTONE_RETENTION_DAYS).await.unwrap();
        assert_eq!(outcome.pruned, 1);
        assert_eq!(outcome.floor, gone_seq, "the floor is how far the prune reached");
        assert_eq!(pruned_through_seq(&pool).await.unwrap(), gone_seq);
        assert_eq!(log_ids(&pool).await, vec!["keep", "later", "recent"]);
    }

    /// The window is a window: a recent tombstone stays readable, or every
    /// client that syncs weekly gets pushed onto a full walk for nothing.
    #[tokio::test]
    async fn a_fresh_tombstone_survives_the_prune() {
        let pool = test_pool().await;
        insert(&pool, "keep", "user-1").await;
        insert(&pool, "just-deleted", "user-1").await;
        sqlx::query("DELETE FROM photos WHERE id = 'just-deleted'")
            .execute(&pool)
            .await
            .unwrap();
        // One day old — well inside a 90-day window, and far enough from `now`
        // that second-resolution rounding cannot make it look aged.
        backdate(&pool, "just-deleted", 1).await;
        // Without this the tombstone is the head and the head guard spares it,
        // so the test would pass with the date window removed entirely — i.e.
        // it would not test the trap it exists for.
        bump_head(&pool, "later").await;

        let outcome = prune_change_log(&pool, TOMBSTONE_RETENTION_DAYS).await.unwrap();
        assert_eq!(outcome.pruned, 0, "a one-day-old tombstone is not 90 days old");
        assert_eq!(outcome.floor, 0, "nothing pruned, so no floor");
        assert_eq!(log_ids(&pool).await, vec!["just-deleted", "keep", "later"]);
    }

    /// **The date-format trap, pinned at the only place it is observable.**
    ///
    /// `changed_at` holds `datetime('now')` (`"… 23:59:59"`, a space) and is
    /// compared as a string. A `chrono` RFC 3339 cutoff (`"…T…+00:00"`, a `T`)
    /// therefore agrees on every row except those dated the *same day* as the
    /// cutoff — where `' '` (0x20) sorts under `'T'` (0x54) and an
    /// inside-the-window row reads as expired.
    ///
    /// So the row is placed on the cutoff's own date, at the last second of it:
    /// strictly newer than a `datetime('now', '-90 days')` cutoff (whose
    /// time-of-day is by construction no later), and therefore spared — but
    /// string-under a same-day RFC 3339 cutoff, and therefore pruned.
    ///
    /// Verified RED by binding `chrono::Utc::now() - Duration::days(90)`
    /// `.to_rfc3339()` against a bare `changed_at < ?1`: this test fails with
    /// the tombstone pruned 12 hours inside its own retention window.
    #[tokio::test]
    async fn a_tombstone_just_inside_the_window_survives() {
        let pool = test_pool().await;
        insert(&pool, "keep", "user-1").await;
        insert(&pool, "boundary", "user-1").await;
        sqlx::query("DELETE FROM photos WHERE id = 'boundary'")
            .execute(&pool)
            .await
            .unwrap();

        // The cutoff's calendar date, at 23:59:59 — later in the day than
        // `datetime('now', '-90 days')` can be, so it is genuinely inside the
        // window no matter what time the suite runs.
        sqlx::query(&format!(
            "UPDATE photo_change_log \
             SET changed_at = date('now', '-{TOMBSTONE_RETENTION_DAYS} days') || ' 23:59:59' \
             WHERE photo_id = 'boundary'"
        ))
        .execute(&pool)
        .await
        .unwrap();
        bump_head(&pool, "later").await;

        let outcome = prune_change_log(&pool, TOMBSTONE_RETENTION_DAYS).await.unwrap();
        assert_eq!(
            outcome.pruned, 0,
            "a tombstone inside the window was pruned — the cutoff is not in the \
             column's own format"
        );
        assert!(log_ids(&pool).await.contains(&"boundary".to_string()));
    }

    /// Live photos keep their rows regardless of age. Migration `033` backfills
    /// one row per photo precisely so `since=0` degenerates into a full sync;
    /// pruning a live photo's row would make `since=0` return a library with
    /// holes in it — a cold-start client silently missing photos.
    #[tokio::test]
    async fn live_photos_keep_their_rows_however_old() {
        let pool = test_pool().await;
        for id in ["a", "b", "c"] {
            insert(&pool, id, "user-1").await;
            backdate(&pool, id, 500).await;
        }

        let outcome = prune_change_log(&pool, TOMBSTONE_RETENTION_DAYS).await.unwrap();
        assert_eq!(outcome.pruned, 0);
        assert_eq!(log_ids(&pool).await, vec!["a", "b", "c"]);
    }

    /// A secure-hidden photo still HAS its `photos` row — only its eligibility
    /// changed. Its change-log entry is the sole instruction telling clients to
    /// drop it, so pruning it would leave a photo the user just secured visible
    /// on every client that had not yet synced.
    #[tokio::test]
    async fn a_secure_hidden_photo_is_not_a_tombstone() {
        let pool = test_pool().await;
        insert(&pool, "secret", "user-1").await;
        insert(&pool, "other", "user-1").await;

        sqlx::query("INSERT OR IGNORE INTO users (id, username, password_hash, created_at) VALUES ('user-1','u','h','2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT OR IGNORE INTO encrypted_galleries (id, user_id, name, password_hash, created_at) VALUES ('g1','user-1','sec','h','2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT OR IGNORE INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) VALUES ('secret','user-1','photo',0,'2026-01-01T00:00:00Z','s')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at) VALUES ('i1','g1','secret','2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();

        backdate(&pool, "secret", 400).await;
        // The EGI insert made `secret` the head; without moving past it the
        // head guard would spare the row and this test would pass even if the
        // tombstone check were deleted.
        bump_head(&pool, "later").await;

        let outcome = prune_change_log(&pool, TOMBSTONE_RETENTION_DAYS).await.unwrap();
        assert_eq!(outcome.pruned, 0, "the photos row still exists — not a tombstone");
        assert!(log_ids(&pool).await.contains(&"secret".to_string()));
    }

    /// **Head monotonicity.** Every trigger computes `MAX(seq) + 1`. Prune the
    /// highest-seq row and the next change reuses a sequence that synced
    /// clients have already passed, so they never see it — a silent, permanent
    /// loss.
    ///
    /// The setup is the realistic one: a library whose most recent event was a
    /// deletion, then months of inactivity.
    ///
    /// Verified RED by dropping the `seq < (SELECT MAX(seq) ...)` arm from
    /// `VICTIM_PREDICATE`: the head falls to 1 and the assertion fires.
    #[tokio::test]
    async fn pruning_never_lowers_the_head() {
        let pool = test_pool().await;
        insert(&pool, "keep", "user-1").await;
        insert(&pool, "last-thing-that-happened", "user-1").await;
        sqlx::query("DELETE FROM photos WHERE id = 'last-thing-that-happened'")
            .execute(&pool)
            .await
            .unwrap();

        let head_before = head(&pool).await;
        let tombstone_seq: i64 = sqlx::query_scalar(
            "SELECT seq FROM photo_change_log WHERE photo_id = 'last-thing-that-happened'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tombstone_seq, head_before, "precondition: the tombstone IS the head");

        // Age everything past the window — the whole log is now "expired".
        backdate(&pool, "keep", 400).await;
        backdate(&pool, "last-thing-that-happened", 400).await;

        let outcome = prune_change_log(&pool, TOMBSTONE_RETENTION_DAYS).await.unwrap();
        assert_eq!(outcome.pruned, 0, "the head row is spared even when expired");
        assert_eq!(head(&pool).await, head_before, "head must never move backwards");
    }

    /// The floor only ever rises. A prune with nothing to take must not reset
    /// it — that would re-validate cursors an earlier prune invalidated, and
    /// serve them a delta missing the tombstones it had already deleted.
    #[tokio::test]
    async fn the_floor_never_falls() {
        let pool = test_pool().await;
        insert(&pool, "keep", "user-1").await;
        insert(&pool, "gone", "user-1").await;
        insert(&pool, "newest", "user-1").await;
        sqlx::query("DELETE FROM photos WHERE id = 'gone'")
            .execute(&pool)
            .await
            .unwrap();
        backdate(&pool, "gone", 200).await;
        bump_head(&pool, "later").await;

        let first = prune_change_log(&pool, TOMBSTONE_RETENTION_DAYS).await.unwrap();
        assert_eq!(first.pruned, 1);
        assert!(first.floor > 0);

        // Second pass: nothing left to take.
        let second = prune_change_log(&pool, TOMBSTONE_RETENTION_DAYS).await.unwrap();
        assert_eq!(second.pruned, 0);
        assert_eq!(second.floor, first.floor, "an empty prune must not reset the floor");
        assert_eq!(pruned_through_seq(&pool).await.unwrap(), first.floor);
    }

    /// A corrupted floor must fail CLOSED — every client onto the full walk —
    /// not open. Reading it as 0 would serve deltas the prune already
    /// invalidated, which is the exact ghost-row bug this module prevents.
    #[tokio::test]
    async fn an_unparseable_floor_invalidates_every_cursor() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO server_settings (key, value) VALUES (?, 'not-a-number')")
            .bind(PRUNED_THROUGH_KEY)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(pruned_through_seq(&pool).await.unwrap(), i64::MAX);
    }
}
