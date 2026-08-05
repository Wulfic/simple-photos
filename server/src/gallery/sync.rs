//! Encrypted-mode sync endpoint.
//!
//! Returns photo metadata from the `photos` table for photos that have been
//! encrypted (have `encrypted_blob_id`). This lets mobile clients populate
//! their gallery without downloading and decrypting every full-size photo blob.
//!
//! Clients then download only the small thumbnail blobs (~30 KB each) for
//! gallery grid display and load full photos on-demand when viewed.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::gallery::eligibility::ELIGIBLE_PREDICATE;
use crate::state::AppState;

/// Columns every sync record is built from. Interpolated rather than repeated:
/// the full walk and the delta feed must project identical shapes, and the two
/// hand-maintained copies that used to live here had already begun to rot.
const RECORD_COLUMNS: &str = "id, filename, mime_type, media_type, size_bytes, width, height, \
     duration_secs, taken_at, created_at, encrypted_blob_id, encrypted_thumb_blob_id, \
     is_favorite, crop_metadata, photo_hash, source_path, \
     photo_subtype, burst_id, motion_video_blob_id";

/// Query parameters for the encrypted sync endpoint.
#[derive(Debug, Deserialize)]
pub struct SyncQuery {
    pub after: Option<String>,
    pub limit: Option<i64>,
    /// Delta mode (#38): return only photos whose change-log sequence exceeds
    /// this, plus tombstones for those that left the feed. Absent = full walk.
    ///
    /// `since=0` is not special-cased. Migration 033 backfills a change-log row
    /// for every pre-existing photo, so `since=0` naturally enumerates the whole
    /// library — a cold-start client and a long-offline client take one path.
    pub since: Option<i64>,
}

/// Photo metadata record for encrypted-mode sync (no file content).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EncryptedSyncRecord {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
    pub created_at: String,
    /// NULL for photos registered by the autoscan pipeline that have not yet
    /// been uploaded as an encrypted blob by a client.
    pub encrypted_blob_id: Option<String>,
    pub encrypted_thumb_blob_id: Option<String>,
    pub is_favorite: bool,
    pub crop_metadata: Option<String>,
    pub photo_hash: Option<String>,
    /// Non-null when this photo was converted from a non-native format.
    /// Contains the relative path to the original file on disk.
    pub source_path: Option<String>,
    pub photo_subtype: Option<String>,
    pub burst_id: Option<String>,
    pub motion_video_blob_id: Option<String>,

    /// Video quality ladder (#49), highest first. Empty for everything that is
    /// not a video above the 1080p tier, which is almost everything.
    ///
    /// Carried on the sync record rather than fetched from a per-video endpoint,
    /// because that is what the change-log triggers in `035`/`036` were built
    /// for: a rung lands minutes-to-hours after its photo, long after every
    /// client has synced it, so the rendition becoming playable *nominates the
    /// photo* and the next delta re-delivers this record. An on-demand endpoint
    /// would make that machinery pointless and put a round trip in front of
    /// every video the user opens.
    ///
    /// Not a DB column — `skip` keeps it out of the projection entirely, and
    /// [`hydrate_renditions`] fills it after the row is read.
    #[sqlx(skip)]
    pub renditions: Vec<crate::transcode::renditions::RenditionDto>,
}

/// Paginated response from `GET /api/photos/encrypted-sync`.
#[derive(Debug, Serialize)]
pub struct EncryptedSyncResponse {
    pub photos: Vec<EncryptedSyncRecord>,
    pub next_cursor: Option<String>,

    /// Ids of photos that have left the feed since the requested sequence —
    /// deleted outright, or claimed by a secure gallery. Delta mode only; the
    /// full walk needs no tombstones because the client set-differences.
    ///
    /// Empty rather than absent so a client cannot mistake "no removals" for
    /// "this server does not send removals".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Vec<String>>,

    /// The change-log head at the time this page was built. Clients persist it
    /// and pass it back as `since` on the next sync.
    ///
    /// **Persist the value from the FIRST page of a walk, not the last.** A
    /// change committed while a multi-page walk is in flight lands at a
    /// sequence above the first page's head, so keeping the first head
    /// re-delivers it next time. Keeping the last head would step over it and
    /// lose the change permanently.
    pub head_seq: i64,
}

/// GET /api/photos/encrypted-sync
/// Returns metadata for encrypted photos — lightweight sync for mobile clients.
///
/// Two modes. Without `since`, the historical full keyset walk over the whole
/// eligible library. With `since`, only what changed after that sequence, plus
/// tombstones. Both paginate with `after`/`limit`, and both report `head_seq`.
pub async fn encrypted_sync(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<SyncQuery>,
) -> Result<Json<EncryptedSyncResponse>, AppError> {
    let limit = params.limit.unwrap_or(500).min(1000);
    let page = match params.since {
        Some(since) => {
            fetch_delta(
                &state.read_pool,
                &auth.user_id,
                since,
                params.after.as_deref(),
                limit,
            )
            .await
        }
        None => {
            fetch_page(
                &state.read_pool,
                &auth.user_id,
                params.after.as_deref(),
                limit,
            )
            .await
        }
    }
    .map_err(|e| {
        tracing::error!(
            user_id = %auth.user_id, since = ?params.since, error = ?e,
            "encrypted_sync page failed"
        );
        e
    })?;
    Ok(Json(page))
}

/// Attach the video quality ladder to a page of records.
///
/// Called by **both** feeds. The full walk and the delta must hand a client
/// identical records — a client cannot tell which path produced a row, and #38
/// treats the full walk as the recovery path for the delta, so a rendition
/// visible through one and not the other would mean a "repair" that silently
/// removes the user's quality picker.
///
/// Costs one query per page, and none at all for a page with no videos.
async fn hydrate_renditions(
    pool: &sqlx::SqlitePool,
    records: &mut [EncryptedSyncRecord],
) -> Result<(), AppError> {
    let video_ids: Vec<&str> = records
        .iter()
        .filter(|r| r.media_type == "video")
        .map(|r| r.id.as_str())
        .collect();

    let mut by_photo =
        crate::transcode::renditions::list_renditions_for_photos(pool, &video_ids).await?;

    for record in records.iter_mut() {
        if let Some(rungs) = by_photo.remove(&record.id) {
            record.renditions = rungs;
        }
    }
    Ok(())
}

/// Current head of the change log — the sequence a client should resume from.
///
/// Global rather than per-user (see migration 033): cheap, monotonic, and a
/// client's cursor only ever needs to be comparable against itself. A user with
/// no changes simply has a sparse range, which costs one empty delta query.
pub async fn head_seq(pool: &sqlx::SqlitePool) -> Result<i64, AppError> {
    let head: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM photo_change_log")
        .fetch_one(pool)
        .await?;
    Ok(head)
}

/// Fetch one keyset page of the encrypted-sync feed.
///
/// Split out of the handler so the pagination contract is unit-testable
/// without an HTTP stack: the round-trip completeness property (every seeded
/// row is returned by exactly one page) is the only thing that catches cursor
/// off-by-ones, and it cannot be asserted through `State`/`AuthUser`.
pub async fn fetch_page(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    after: Option<&str>,
    limit: i64,
) -> Result<EncryptedSyncResponse, AppError> {
    // Cursor format: "timestamp|id" for keyset pagination.
    // Using (timestamp, id) as a composite key avoids skipping items that
    // share the same timestamp (e.g. batch-converted files).
    let mut photos = if let Some(after) = after {
        let (cursor_ts, cursor_id) = if let Some(idx) = after.rfind('|') {
            (after[..idx].to_string(), after[idx + 1..].to_string())
        } else {
            // Legacy cursor (timestamp only) — use empty id so all items
            // at the boundary timestamp are included via <=.
            (after.to_string(), String::new())
        };
        sqlx::query_as::<_, EncryptedSyncRecord>(&format!(
            "SELECT {RECORD_COLUMNS} \
             FROM photos p \
             WHERE p.user_id = ? \
             AND {ELIGIBLE_PREDICATE} \
             AND (COALESCE(p.taken_at, p.created_at) < ? \
                  OR (COALESCE(p.taken_at, p.created_at) = ? AND p.id > ?)) \
             ORDER BY COALESCE(p.taken_at, p.created_at) DESC, p.id ASC \
             LIMIT ?"
        ))
        .bind(user_id)
        .bind(&cursor_ts)
        .bind(&cursor_ts)
        .bind(&cursor_id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, EncryptedSyncRecord>(&format!(
            "SELECT {RECORD_COLUMNS} \
             FROM photos p \
             WHERE p.user_id = ? \
             AND {ELIGIBLE_PREDICATE} \
             ORDER BY COALESCE(p.taken_at, p.created_at) DESC, p.id ASC \
             LIMIT ?"
        ))
        .bind(user_id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?
    };

    // We fetched `limit + 1` purely to detect whether another page exists.
    // Truncate FIRST, then derive the cursor from the last row we actually
    // return. Deriving it from the extra peeked row (as this did previously)
    // loses that row permanently: the next page's predicate is strict
    // (`< ts OR (= ts AND id > id)`), so the row named by the cursor is never
    // returned by any page — one photo silently vanished per page boundary.
    let has_more = photos.len() as i64 > limit;
    photos.truncate(limit as usize);

    let next_cursor = if has_more {
        photos.last().map(|p| {
            let ts = p.taken_at.clone().unwrap_or_else(|| p.created_at.clone());
            format!("{}|{}", ts, p.id)
        })
    } else {
        None
    };

    // After truncation, so a peeked row never costs a rendition lookup.
    hydrate_renditions(pool, &mut photos).await?;

    Ok(EncryptedSyncResponse {
        photos,
        next_cursor,
        deleted: None,
        head_seq: head_seq(pool).await?,
    })
}

/// Fetch one page of the **delta** feed: everything that changed after `since`.
///
/// ## Why this is safe despite nine `DELETE FROM photos` sites
///
/// `photo_change_log` is a hint, not a source of truth. Its triggers say only
/// "photo X may have changed"; they never claim X was deleted or is eligible.
/// This function re-derives both from the live tables using
/// [`ELIGIBLE_PREDICATE`] — the exact predicate [`fetch_page`] uses. So the
/// upsert/tombstone split is always computed against current reality, and a
/// trigger that fires spuriously costs one redundant row, never a wrong answer.
///
/// That is what lets a single trigger cover every delete site at once, and what
/// makes the secure-album case work at all: adding a photo to a secure gallery
/// never touches its `photos` row, so no amount of care on the `photos` table
/// alone could have caught it.
///
/// ## Pagination
///
/// The cursor is composite — `"<seq>|<photo_id>"` — and NOT a bare `seq`.
/// Sequences are **not unique**: `MAX(seq) + 1` is evaluated once per statement,
/// so when one secure-gallery insert touches several photos they all land on the
/// same sequence. A `seq > last` cursor would drop every member of such a group
/// after the first whenever a page boundary fell inside it — the identical
/// off-by-one that lost one photo per page in #42, reintroduced in a new place.
/// Ordering by `(seq, photo_id)` and comparing lexicographically makes the
/// boundary exact. `rows_sharing_a_sequence_survive_a_page_boundary` covers it.
///
/// ## Retention floor
///
/// Tombstones do not live forever (`gallery::retention`). A `since` below the
/// pruned-through floor cannot be answered: some removal in `(since, floor]`
/// has been deleted, so a delta built from what remains would silently omit it
/// and the client would keep that photo forever. Such a request is answered
/// with the **full walk** instead — self-healing, and already the recovery
/// branch both clients take when `deleted` comes back absent.
pub async fn fetch_delta(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    since: i64,
    after: Option<&str>,
    limit: i64,
) -> Result<EncryptedSyncResponse, AppError> {
    // Checked before any delta work: below the floor there is no correct delta
    // to build, only a convincing wrong one.
    let floor = crate::gallery::retention::pruned_through_seq(pool).await?;
    if since < floor {
        tracing::info!(
            user_id = %user_id, since, floor,
            "delta cursor predates the retention floor; answering with a full walk"
        );
        // `after` is deliberately dropped. A delta cursor is `"<seq>|<id>"`,
        // which means nothing to the full walk's `"<timestamp>|<id>"` keyset —
        // and neither client resumes here anyway: an absent `deleted` makes web
        // discard the partial delta and call `runFullPass`, and Android reset
        // `after = null` and restart. Handing back page one of a coherent full
        // walk is the only answer either of them can use.
        return fetch_page(pool, user_id, None, limit).await;
    }

    // Capture the head BEFORE reading, so a change landing mid-page is
    // re-delivered next time rather than stepped over.
    let head = head_seq(pool).await?;

    // Step 1: page the change log. This decides the page boundary and the
    // cursor; eligibility is deliberately not consulted yet, because a photo
    // that became INELIGIBLE still has to occupy a slot on this page — it is
    // the tombstone.
    let mut changed: Vec<ChangedRow> = if let Some(after) = after {
        let (cur_seq, cur_id) = parse_delta_cursor(after);
        sqlx::query_as::<_, ChangedRow>(
            "SELECT photo_id, seq FROM photo_change_log \
             WHERE user_id = ? AND seq > ? \
               AND (seq > ? OR (seq = ? AND photo_id > ?)) \
             ORDER BY seq ASC, photo_id ASC \
             LIMIT ?",
        )
        .bind(user_id)
        .bind(since)
        .bind(cur_seq)
        .bind(cur_seq)
        .bind(&cur_id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ChangedRow>(
            "SELECT photo_id, seq FROM photo_change_log \
             WHERE user_id = ? AND seq > ? \
             ORDER BY seq ASC, photo_id ASC \
             LIMIT ?",
        )
        .bind(user_id)
        .bind(since)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?
    };

    // Same peek-then-truncate discipline as `fetch_page`, and for the same
    // reason: derive the cursor from the last row actually RETURNED.
    let has_more = changed.len() as i64 > limit;
    changed.truncate(limit as usize);
    let next_cursor = if has_more {
        changed.last().map(|c| format!("{}|{}", c.seq, c.photo_id))
    } else {
        None
    };

    if changed.is_empty() {
        // The steady state: nothing changed, so no rows and no blob work. This
        // is the whole point of #38 — assert it in tests, it is the property
        // that regresses silently.
        return Ok(EncryptedSyncResponse {
            photos: Vec::new(),
            next_cursor: None,
            deleted: Some(Vec::new()),
            head_seq: head,
        });
    }

    // Step 2: of this page's ids, fetch the ones that are currently eligible.
    // Anything the query does not return is, by definition, no longer in the
    // feed — deleted or secure-hidden, which the client treats identically.
    let ids: Vec<&str> = changed.iter().map(|c| c.photo_id.as_str()).collect();
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "SELECT {RECORD_COLUMNS} FROM photos p \
         WHERE p.id IN ({placeholders}) AND p.user_id = ? AND {ELIGIBLE_PREDICATE} \
         ORDER BY COALESCE(p.taken_at, p.created_at) DESC, p.id ASC"
    );
    let mut q = sqlx::query_as::<_, EncryptedSyncRecord>(&sql);
    for id in &ids {
        q = q.bind(*id);
    }
    let mut photos = q.bind(user_id).fetch_all(pool).await?;
    hydrate_renditions(pool, &mut photos).await?;

    let alive: std::collections::HashSet<&str> = photos.iter().map(|p| p.id.as_str()).collect();
    let deleted: Vec<String> = changed
        .iter()
        .filter(|c| !alive.contains(c.photo_id.as_str()))
        .map(|c| c.photo_id.clone())
        .collect();

    Ok(EncryptedSyncResponse {
        photos,
        next_cursor,
        deleted: Some(deleted),
        head_seq: head,
    })
}

/// One change-log entry: which photo, and at what sequence.
#[derive(Debug, sqlx::FromRow)]
struct ChangedRow {
    photo_id: String,
    seq: i64,
}

/// Split a `"<seq>|<photo_id>"` delta cursor. A malformed or unparseable
/// cursor degrades to `(0, "")`, which restarts the delta from `since` — a
/// duplicate page, never a skipped one.
fn parse_delta_cursor(after: &str) -> (i64, String) {
    match after.split_once('|') {
        Some((seq, id)) => (seq.parse().unwrap_or(0), id.to_string()),
        None => (after.parse().unwrap_or(0), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::str::FromStr;

    // See `summary.rs` for why max_connections(1) + foreign_keys(false):
    // a pool to `sqlite::memory:` otherwise gives each connection its own
    // empty DB, and we insert bare photo rows without the users/blobs graph.
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

    async fn insert(pool: &sqlx::SqlitePool, id: &str, user: &str, taken_at: &str) {
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
        .bind(taken_at)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Walk every page the way a real client does — follow `next_cursor` until
    /// it comes back `None` — and collect the ids in page order.
    async fn paginate_all(pool: &sqlx::SqlitePool, user: &str, limit: i64) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        // Safety valve: a cursor bug that fails to advance must fail the test,
        // not hang the suite.
        for _ in 0..100 {
            let page = fetch_page(pool, user, cursor.as_deref(), limit)
                .await
                .unwrap();
            ids.extend(page.photos.iter().map(|p| p.id.clone()));
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => return ids,
            }
        }
        panic!("pagination did not terminate — cursor is not advancing");
    }

    fn assert_round_trip(got: &[String], seeded: &[String]) {
        let got_set: HashSet<&String> = got.iter().collect();
        let want_set: HashSet<&String> = seeded.iter().collect();

        let missing: Vec<_> = want_set.difference(&got_set).collect();
        assert!(
            missing.is_empty(),
            "rows were never returned by ANY page: {missing:?}"
        );
        assert_eq!(
            got.len(),
            seeded.len(),
            "expected each row exactly once, got {} for {} seeded (duplicates?)",
            got.len(),
            seeded.len()
        );
        assert_eq!(got_set, want_set);
    }

    /// The regression: with `limit + 1` fetched to detect a next page, the
    /// cursor used to be built from the extra peeked row, which the strict
    /// next-page predicate then excluded. Exactly one row vanished per page
    /// boundary — invisibly, because nothing asserted completeness.
    #[tokio::test]
    async fn every_seeded_row_is_returned_exactly_once() {
        let pool = test_pool().await;
        let u = "user-1";

        // Distinct, descending-friendly timestamps so ordering is unambiguous.
        let mut seeded = Vec::new();
        for i in 0..7 {
            let id = format!("p{i:02}");
            insert(&pool, &id, u, &format!("2026-01-{:02}T00:00:00Z", i + 1)).await;
            seeded.push(id);
        }

        // limit + 1 (4 rows over a limit of 3) — a single page boundary.
        let got = paginate_all(&pool, u, 3).await;
        assert_round_trip(&got, &seeded);

        // A limit that divides evenly still has a boundary after the last page.
        let got = paginate_all(&pool, u, 7).await;
        assert_round_trip(&got, &seeded);

        // limit of 1 maximises the number of boundaries — 6 chances to drop.
        let got = paginate_all(&pool, u, 1).await;
        assert_round_trip(&got, &seeded);
    }

    /// Rows sharing a timestamp are what the composite `ts|id` cursor exists
    /// for. Batch-converted imports produce these in bulk, so a page boundary
    /// landing inside a tie group must not drop the group.
    #[tokio::test]
    async fn rows_sharing_a_timestamp_survive_a_page_boundary() {
        let pool = test_pool().await;
        let u = "user-1";

        let mut seeded = Vec::new();
        for i in 0..5 {
            let id = format!("t{i:02}");
            // Every row has the SAME taken_at — ordering falls entirely to id.
            insert(&pool, &id, u, "2026-02-01T00:00:00Z").await;
            seeded.push(id);
        }

        for limit in [1, 2, 3, 4, 5] {
            let got = paginate_all(&pool, u, limit).await;
            assert_round_trip(&got, &seeded);
        }
    }

    // ── Delta feed (#38) ───────────────────────────────────────────────────

    /// Seed the FK parents a secure-gallery item needs, then hide `photo_id`
    /// inside a secure gallery. Mirrors what `gallery::secure` does: it inserts
    /// an `encrypted_gallery_items` row and never touches `photos`.
    async fn secure_hide(pool: &sqlx::SqlitePool, item: &str, photo_id: &str) {
        sqlx::query("INSERT OR IGNORE INTO users (id, username, password_hash, created_at) VALUES ('user-1','u','h','2026-01-01T00:00:00Z')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT OR IGNORE INTO encrypted_galleries (id, user_id, name, password_hash, created_at) VALUES ('g1','user-1','sec','h','2026-01-01T00:00:00Z')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT OR IGNORE INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) VALUES (?,'user-1','photo',0,'2026-01-01T00:00:00Z','s')")
            .bind(photo_id).execute(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, added_at) VALUES (?, 'g1', ?, '2026-01-01T00:00:00Z')",
        )
        .bind(item)
        .bind(photo_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Walk the delta feed to exhaustion, returning (upserted ids, tombstones).
    async fn delta_all(
        pool: &sqlx::SqlitePool,
        user: &str,
        since: i64,
        limit: i64,
    ) -> (Vec<String>, Vec<String>) {
        let (mut up, mut del) = (Vec::new(), Vec::new());
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let page = fetch_delta(pool, user, since, cursor.as_deref(), limit)
                .await
                .unwrap();
            up.extend(page.photos.iter().map(|p| p.id.clone()));
            del.extend(page.deleted.clone().unwrap_or_default());
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => return (up, del),
            }
        }
        panic!("delta pagination did not terminate — cursor is not advancing");
    }

    /// The headline property: an unchanged library transfers NOTHING. This is
    /// the entire point of #38, and it is the property that will regress
    /// silently, because a client that re-downloads everything still shows the
    /// correct gallery — just slowly, exactly as before.
    #[tokio::test]
    async fn steady_state_delta_is_empty() {
        let pool = test_pool().await;
        for i in 0..20 {
            insert(&pool, &format!("s{i:02}"), "user-1", "2026-01-01T00:00:00Z").await;
        }
        let head = head_seq(&pool).await.unwrap();

        let page = fetch_delta(&pool, "user-1", head, None, 500).await.unwrap();
        assert!(
            page.photos.is_empty(),
            "no rows should move when nothing changed"
        );
        assert_eq!(page.deleted, Some(Vec::new()));
        assert_eq!(page.next_cursor, None, "no pagination for an empty delta");
        assert_eq!(page.head_seq, head, "head must not drift on a read");
    }

    /// Migration 033 backfills every pre-existing photo, so `since = 0` must be
    /// indistinguishable from a full walk. That is what lets a cold-start
    /// client and a long-offline client share one code path.
    #[tokio::test]
    async fn since_zero_returns_the_whole_library() {
        let pool = test_pool().await;
        let mut seeded = Vec::new();
        for i in 0..7 {
            let id = format!("z{i:02}");
            insert(
                &pool,
                &id,
                "user-1",
                &format!("2026-01-{:02}T00:00:00Z", i + 1),
            )
            .await;
            seeded.push(id);
        }

        let full = paginate_all(&pool, "user-1", 3).await;
        let (delta, tombs) = delta_all(&pool, "user-1", 0, 3).await;

        assert!(tombs.is_empty(), "nothing was removed");
        assert_round_trip(&delta, &seeded);
        assert_eq!(
            delta.iter().collect::<HashSet<_>>(),
            full.iter().collect::<HashSet<_>>(),
            "delta from 0 must equal the full walk"
        );
    }

    /// A deleted photo must be named explicitly. The full walk was self-healing
    /// — it simply stopped returning the row and the client set-differenced it
    /// away. A delta feed that fails to emit the tombstone leaves a ghost row
    /// in every client forever, which is why #38 was a regression risk.
    #[tokio::test]
    async fn deleting_a_photo_emits_a_tombstone() {
        let pool = test_pool().await;
        insert(&pool, "keep", "user-1", "2026-01-01T00:00:00Z").await;
        insert(&pool, "gone", "user-1", "2026-01-02T00:00:00Z").await;
        let head = head_seq(&pool).await.unwrap();

        sqlx::query("DELETE FROM photos WHERE id = 'gone'")
            .execute(&pool)
            .await
            .unwrap();

        let (up, del) = delta_all(&pool, "user-1", head, 500).await;
        assert!(up.is_empty(), "nothing was added or modified");
        assert_eq!(del, vec!["gone".to_string()]);
    }

    /// The case a `photos`-only trigger cannot see, and the reason this design
    /// watches `encrypted_gallery_items` too: securing a photo removes it from
    /// the feed WITHOUT updating, or even touching, its `photos` row.
    #[tokio::test]
    async fn securing_a_photo_emits_a_tombstone_without_touching_the_row() {
        let pool = test_pool().await;
        insert(&pool, "keep", "user-1", "2026-01-01T00:00:00Z").await;
        insert(&pool, "secret", "user-1", "2026-01-02T00:00:00Z").await;
        let head = head_seq(&pool).await.unwrap();

        secure_hide(&pool, "i1", "secret").await;

        // Precondition: the photo row itself is untouched.
        let still_there: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE id = 'secret'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            still_there, 1,
            "the row must still exist — only eligibility changed"
        );

        let (up, del) = delta_all(&pool, "user-1", head, 500).await;
        assert!(up.is_empty());
        assert_eq!(
            del,
            vec!["secret".to_string()],
            "secure-hidden reads as removed"
        );
    }

    /// The mirror image: removing the secure-gallery item puts the photo back
    /// in the feed, and the delta must re-deliver the full record so the client
    /// can render it again.
    #[tokio::test]
    async fn unsecuring_a_photo_re_delivers_it() {
        let pool = test_pool().await;
        insert(&pool, "secret", "user-1", "2026-01-02T00:00:00Z").await;
        secure_hide(&pool, "i1", "secret").await;
        let head = head_seq(&pool).await.unwrap();

        sqlx::query("DELETE FROM encrypted_gallery_items WHERE id = 'i1'")
            .execute(&pool)
            .await
            .unwrap();

        let (up, del) = delta_all(&pool, "user-1", head, 500).await;
        assert_eq!(
            up,
            vec!["secret".to_string()],
            "back in the feed as an upsert"
        );
        assert!(del.is_empty());
    }

    /// Deleting a whole secure gallery removes its items by ON DELETE CASCADE,
    /// never by an explicit `DELETE FROM encrypted_gallery_items`. SQLite fires
    /// AFTER DELETE triggers for cascaded rows — this test pins that, because
    /// the entire tombstone design collapses if a future SQLite (or a schema
    /// change) stops firing them.
    #[tokio::test]
    async fn cascade_deleting_a_gallery_re_delivers_its_photos() {
        let pool = test_pool().await;
        insert(&pool, "secret", "user-1", "2026-01-02T00:00:00Z").await;
        secure_hide(&pool, "i1", "secret").await;
        let head = head_seq(&pool).await.unwrap();

        // Cascade path — note this never names encrypted_gallery_items.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM encrypted_galleries WHERE id = 'g1'")
            .execute(&pool)
            .await
            .unwrap();

        let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM encrypted_gallery_items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            items, 0,
            "precondition: the cascade actually removed the item"
        );

        let (up, _) = delta_all(&pool, "user-1", head, 500).await;
        assert_eq!(
            up,
            vec!["secret".to_string()],
            "cascade must fire the trigger, else the photo stays invisible forever"
        );
    }

    /// Sequences are NOT unique: `MAX(seq) + 1` is evaluated once per
    /// statement, so one secure-gallery insert touching several photos lands
    /// them all on the same sequence. A bare `seq > last` cursor drops every
    /// member of the group after the first when a page boundary falls inside
    /// it — the same class of off-by-one that lost a photo per page in #42.
    ///
    /// `limit = 1` maximises boundaries, so this fails loudly if the composite
    /// `(seq, photo_id)` cursor is ever simplified back to a bare sequence.
    #[tokio::test]
    async fn rows_sharing_a_sequence_survive_a_page_boundary() {
        let pool = test_pool().await;
        // Three photos all pointing at one encrypted blob, so a single EGI
        // insert matches all three at once.
        for id in ["m1", "m2", "m3"] {
            insert(&pool, id, "user-1", "2026-01-01T00:00:00Z").await;
            sqlx::query("UPDATE photos SET encrypted_blob_id = 'shared-blob' WHERE id = ?")
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let head = head_seq(&pool).await.unwrap();

        sqlx::query("INSERT OR IGNORE INTO users (id, username, password_hash, created_at) VALUES ('user-1','u','h','2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT OR IGNORE INTO encrypted_galleries (id, user_id, name, password_hash, created_at) VALUES ('g1','user-1','sec','h','2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT OR IGNORE INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) VALUES ('shared-blob','user-1','photo',0,'2026-01-01T00:00:00Z','s')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO encrypted_gallery_items (id, gallery_id, blob_id, original_blob_id, added_at) VALUES ('i1','g1','shared-blob','shared-blob','2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();

        // Precondition: they really do share one sequence.
        let distinct: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT seq) FROM photo_change_log WHERE photo_id IN ('m1','m2','m3')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(distinct, 1, "precondition: one statement, one sequence");

        for limit in [1, 2, 3, 5] {
            let (_, del) = delta_all(&pool, "user-1", head, limit).await;
            let got: HashSet<&String> = del.iter().collect();
            let want = ["m1".to_string(), "m2".to_string(), "m3".to_string()];
            let want: HashSet<&String> = want.iter().collect();
            assert_eq!(got, want, "tombstones lost at limit={limit}");
        }
    }

    /// The invariant tying the two feeds together: applying the delta to a
    /// mirror must land on exactly the same set as a fresh full walk. This is
    /// the test that would catch the delta and the full walk drifting apart —
    /// e.g. someone editing the eligibility predicate in only one of them.
    #[tokio::test]
    async fn applying_the_delta_matches_a_fresh_full_walk() {
        let pool = test_pool().await;
        let u = "user-1";
        for i in 0..6 {
            insert(
                &pool,
                &format!("w{i:02}"),
                u,
                &format!("2026-01-{:02}T00:00:00Z", i + 1),
            )
            .await;
        }

        // A client that has fully synced holds exactly the full walk.
        let mut mirror: HashSet<String> = paginate_all(&pool, u, 2).await.into_iter().collect();
        let head = head_seq(&pool).await.unwrap();

        // Now churn the library through every mutation kind at once.
        sqlx::query("DELETE FROM photos WHERE id = 'w00'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE photos SET is_favorite = 1 WHERE id = 'w01'")
            .execute(&pool)
            .await
            .unwrap();
        insert(&pool, "w99", u, "2026-02-01T00:00:00Z").await;
        secure_hide(&pool, "i1", "w02").await;

        // Apply the delta the way a client would.
        let (up, del) = delta_all(&pool, u, head, 2).await;
        for id in del {
            mirror.remove(&id);
        }
        mirror.extend(up);

        let fresh: HashSet<String> = paginate_all(&pool, u, 2).await.into_iter().collect();
        assert_eq!(
            mirror, fresh,
            "delta-applied mirror diverged from a full walk"
        );
    }

    /// Tombstones outlive the rows they describe — that is their entire job —
    /// so they must still be attributable to the right user afterwards.
    /// Leaking one user's deletions into another's feed would be a privacy bug,
    /// not merely a correctness one.
    #[tokio::test]
    async fn delta_is_scoped_to_the_requesting_user() {
        let pool = test_pool().await;
        insert(&pool, "mine", "user-1", "2026-01-01T00:00:00Z").await;
        insert(&pool, "theirs", "user-2", "2026-01-01T00:00:00Z").await;
        let head = head_seq(&pool).await.unwrap();

        sqlx::query("DELETE FROM photos WHERE id = 'theirs'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE photos SET is_favorite = 1 WHERE id = 'mine'")
            .execute(&pool)
            .await
            .unwrap();

        let (up, del) = delta_all(&pool, "user-1", head, 500).await;
        assert_eq!(up, vec!["mine".to_string()]);
        assert!(
            del.is_empty(),
            "another user's tombstone must not leak: {del:?}"
        );
    }

    /// A malformed cursor must re-serve a page, never skip one. Cursors survive
    /// client restarts and storage round-trips, so "unparseable" is a real
    /// state, and silently advancing past it would lose rows invisibly.
    #[test]
    fn malformed_delta_cursors_restart_rather_than_skip() {
        assert_eq!(parse_delta_cursor("12|abc"), (12, "abc".to_string()));
        assert_eq!(parse_delta_cursor("12"), (12, String::new()));
        assert_eq!(parse_delta_cursor("garbage"), (0, String::new()));
        assert_eq!(parse_delta_cursor(""), (0, String::new()));
        // An id containing the separator must split on the FIRST '|' so the
        // sequence parses; the remainder stays part of the id.
        assert_eq!(parse_delta_cursor("7|a|b"), (7, "a|b".to_string()));
    }

    // ── Retention floor (#38 A2) ───────────────────────────────────────────

    /// Id of the photo added purely to move the change-log head past the
    /// tombstone under test. It shows up in every expectation below, so it is
    /// named rather than anonymous.
    const AFTER_VICTIM: &str = "after-the-victim";

    /// Age a tombstone past the window and prune it, returning the new floor.
    ///
    /// The extra insert is not scaffolding: a deletion is the most recent event
    /// at the moment it happens, so its tombstone is the head, and the head is
    /// never pruned (it seeds every trigger's `MAX(seq) + 1`). A tombstone only
    /// becomes prunable once some later change exists — which is exactly the
    /// real-world condition, and without it every test here would prune nothing.
    async fn prune_after_deleting(pool: &sqlx::SqlitePool, photo_id: &str) -> i64 {
        sqlx::query("DELETE FROM photos WHERE id = ?")
            .bind(photo_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE photo_change_log SET changed_at = datetime('now', '-200 days') WHERE photo_id = ?")
            .bind(photo_id)
            .execute(pool)
            .await
            .unwrap();
        insert(pool, AFTER_VICTIM, "user-1", "2026-06-01T00:00:00Z").await;

        let outcome = crate::gallery::retention::prune_change_log(
            pool,
            crate::gallery::retention::TOMBSTONE_RETENTION_DAYS,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome.pruned, 1,
            "precondition: the tombstone was actually pruned"
        );
        outcome.floor
    }

    /// **The whole point of A2.** Once a tombstone is gone, a client whose
    /// cursor predates it can never be told that photo left the feed — no
    /// future response mentions the id again. Serving such a client a delta
    /// looks fine and leaves a ghost row in its mirror forever, so the request
    /// must be answered with the self-healing full walk instead.
    ///
    /// Verified RED by removing the floor check from `fetch_delta`: the delta
    /// comes back with `deleted: Some([])` and the pruned photo absent from
    /// `photos`, i.e. the client is told nothing at all about it.
    #[tokio::test]
    async fn a_cursor_below_the_retention_floor_gets_a_full_walk() {
        let pool = test_pool().await;
        insert(&pool, "alive", "user-1", "2026-01-01T00:00:00Z").await;
        insert(&pool, "erased", "user-1", "2026-01-02T00:00:00Z").await;
        insert(&pool, "newest", "user-1", "2026-01-03T00:00:00Z").await;

        let floor = prune_after_deleting(&pool, "erased").await;
        assert!(floor > 0);

        let page = fetch_delta(&pool, "user-1", floor - 1, None, 500)
            .await
            .unwrap();

        // The handshake both clients key off. `deleted` absent is what makes
        // web's runDeltaPass return null and Android restart with after=null.
        assert_eq!(
            page.deleted, None,
            "an absent `deleted` is the ONLY signal a client has that this is a full walk"
        );
        // And it really is the whole library, not an empty delta wearing a
        // full walk's shape.
        let ids: HashSet<&str> = page.photos.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, HashSet::from(["alive", "newest", AFTER_VICTIM]));
    }

    /// A client that is already at or above the floor saw every pruned
    /// tombstone before it was pruned, so it still gets the cheap path. If this
    /// regresses, the retention policy has quietly turned every sync back into
    /// a full library walk — #38 undone.
    #[tokio::test]
    async fn a_cursor_at_the_retention_floor_still_gets_a_delta() {
        let pool = test_pool().await;
        insert(&pool, "alive", "user-1", "2026-01-01T00:00:00Z").await;
        insert(&pool, "erased", "user-1", "2026-01-02T00:00:00Z").await;
        insert(&pool, "newest", "user-1", "2026-01-03T00:00:00Z").await;

        let floor = prune_after_deleting(&pool, "erased").await;

        let page = fetch_delta(&pool, "user-1", floor, None, 500)
            .await
            .unwrap();
        assert_eq!(
            page.deleted,
            Some(Vec::new()),
            "still a delta, not a full walk"
        );
        // Exactly what changed after the floor — not the whole library.
        let ids: Vec<&str> = page.photos.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec![AFTER_VICTIM]);

        // The steady state must still cost nothing.
        let head = head_seq(&pool).await.unwrap();
        let page = fetch_delta(&pool, "user-1", head, None, 500).await.unwrap();
        assert_eq!(page.deleted, Some(Vec::new()));
        assert!(page.photos.is_empty());
    }

    /// A server that has never pruned must behave exactly as it did before
    /// retention existed — floor 0, every cursor honoured, including `since=0`.
    #[tokio::test]
    async fn an_unpruned_server_honours_every_cursor() {
        let pool = test_pool().await;
        for i in 0..4 {
            insert(&pool, &format!("n{i}"), "user-1", "2026-01-01T00:00:00Z").await;
        }
        let page = fetch_delta(&pool, "user-1", 0, None, 500).await.unwrap();
        assert!(
            page.deleted.is_some(),
            "no prune has run, so since=0 is still a delta"
        );
        assert_eq!(page.photos.len(), 4);
    }

    /// The recovery has to actually recover. A client stuck below the floor
    /// takes the full walk and set-differences — and must land on exactly the
    /// same mirror a healthy delta client holds, pruned photo included.
    #[tokio::test]
    async fn the_full_walk_fallback_repairs_a_stale_mirror() {
        let pool = test_pool().await;
        let u = "user-1";
        for i in 0..4 {
            insert(
                &pool,
                &format!("r{i}"),
                u,
                &format!("2026-01-0{}T00:00:00Z", i + 1),
            )
            .await;
        }
        // A client that synced when everything existed.
        let mut mirror: HashSet<String> = paginate_all(&pool, u, 2).await.into_iter().collect();
        let stale_cursor = head_seq(&pool).await.unwrap();

        let floor = prune_after_deleting(&pool, "r0").await;
        assert!(
            stale_cursor < floor,
            "precondition: this client is now below the floor"
        );

        // It asks for a delta and is handed a full walk, so it set-differences
        // exactly as `runFullPass` does.
        let page = fetch_delta(&pool, u, stale_cursor, None, 500)
            .await
            .unwrap();
        assert_eq!(page.deleted, None);
        let served: HashSet<String> = page.photos.iter().map(|p| p.id.clone()).collect();
        mirror.retain(|id| served.contains(id));
        mirror.extend(served);

        let fresh: HashSet<String> = paginate_all(&pool, u, 2).await.into_iter().collect();
        assert_eq!(
            mirror, fresh,
            "the fallback must leave the mirror exactly correct"
        );
        assert!(
            !mirror.contains("r0"),
            "the pruned photo must be gone from the mirror"
        );
    }

    // ── Video quality ladder (#49) ─────────────────────────────────────────

    /// Register a video with a produced 1080p rung, the way the ladder does.
    async fn insert_video_with_rung(pool: &sqlx::SqlitePool, id: &str, user: &str) {
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, created_at, taken_at, is_favorite) \
             VALUES (?, ?, ?, '', 'video/mp4', 'video', 0, 3840, 2160, \
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0)",
        )
        .bind(id)
        .bind(user)
        .bind(format!("{id}.mp4"))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO video_renditions (photo_id, short_edge, width, height, is_source, \
             blob_id, size_bytes, created_at) \
             VALUES (?, 1080, 1920, 1080, 0, ?, 2048, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("rb-{id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    /// **The drift test.** A client cannot tell which feed produced a record,
    /// and #38 treats the full walk as the *recovery* path for the delta — so a
    /// ladder visible through one and not the other means a "repair" pass that
    /// silently strips the user's quality picker.
    ///
    /// Verified RED by hydrating only `fetch_page`: the delta returns the same
    /// video with an empty ladder.
    #[tokio::test]
    async fn both_feeds_carry_the_video_quality_ladder() {
        let pool = test_pool().await;
        insert_video_with_rung(&pool, "vid", "user-1").await;
        insert(&pool, "still", "user-1", "2026-01-02T00:00:00Z").await;

        let full = fetch_page(&pool, "user-1", None, 500).await.unwrap();
        let delta = fetch_delta(&pool, "user-1", 0, None, 500).await.unwrap();

        for (label, page) in [("full walk", &full), ("delta", &delta)] {
            let video = page
                .photos
                .iter()
                .find(|p| p.id == "vid")
                .unwrap_or_else(|| panic!("{label} did not return the video"));
            assert_eq!(
                video.renditions.len(),
                1,
                "{label} must carry the ladder, got {:?}",
                video.renditions
            );
            assert_eq!(video.renditions[0].short_edge, 1080);
            assert_eq!(video.renditions[0].blob_id.as_deref(), Some("rb-vid"));

            let still = page.photos.iter().find(|p| p.id == "still").unwrap();
            assert!(
                still.renditions.is_empty(),
                "{label} gave a still a quality ladder"
            );
        }
    }

    /// A rung becoming playable nominates its photo (migrations 035/036), and
    /// the point of that nomination is that the *next delta* delivers the
    /// picker. Without this the ladder is invisible until a full walk which,
    /// post-#38, may never happen.
    #[tokio::test]
    async fn a_new_rung_reaches_a_synced_client_through_the_delta() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, created_at, taken_at, is_favorite) \
             VALUES ('vid', 'user-1', 'v.mp4', '', 'video/mp4', 'video', 0, 3840, 2160, \
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        // A client that has fully synced, before any rung exists.
        let synced = fetch_page(&pool, "user-1", None, 500).await.unwrap();
        assert!(synced.photos[0].renditions.is_empty());
        let head = head_seq(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO video_renditions (photo_id, short_edge, width, height, is_source, \
             blob_id, size_bytes, created_at) \
             VALUES ('vid', 1080, 1920, 1080, 0, 'rb1', 2048, '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let (up, del) = delta_all(&pool, "user-1", head, 500).await;
        assert!(del.is_empty());
        assert_eq!(
            up,
            vec!["vid".to_string()],
            "the rung must nominate the photo"
        );

        let page = fetch_delta(&pool, "user-1", head, None, 500).await.unwrap();
        assert_eq!(
            page.photos[0].renditions.len(),
            1,
            "picker must arrive with it"
        );
    }

    /// Another user's rows must never leak into a page, and must not consume
    /// page capacity.
    #[tokio::test]
    async fn pagination_is_scoped_to_the_requesting_user() {
        let pool = test_pool().await;
        insert(&pool, "mine-1", "user-1", "2026-03-01T00:00:00Z").await;
        insert(&pool, "mine-2", "user-1", "2026-03-02T00:00:00Z").await;
        insert(&pool, "theirs", "user-2", "2026-03-03T00:00:00Z").await;

        let got = paginate_all(&pool, "user-1", 1).await;
        assert_round_trip(&got, &["mine-1".to_string(), "mine-2".to_string()]);
    }
}
