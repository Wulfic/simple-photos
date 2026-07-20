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
use crate::state::AppState;

/// Query parameters for the encrypted sync endpoint.
#[derive(Debug, Deserialize)]
pub struct SyncQuery {
    pub after: Option<String>,
    pub limit: Option<i64>,
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
}

/// Paginated response from `GET /api/photos/encrypted-sync`.
#[derive(Debug, Serialize)]
pub struct EncryptedSyncResponse {
    pub photos: Vec<EncryptedSyncRecord>,
    pub next_cursor: Option<String>,
}

/// GET /api/photos/encrypted-sync
/// Returns metadata for encrypted photos — lightweight sync for mobile clients.
pub async fn encrypted_sync(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<SyncQuery>,
) -> Result<Json<EncryptedSyncResponse>, AppError> {
    let limit = params.limit.unwrap_or(500).min(1000);
    let page = fetch_page(
        &state.read_pool,
        &auth.user_id,
        params.after.as_deref(),
        limit,
    )
    .await?;
    Ok(Json(page))
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
        sqlx::query_as::<_, EncryptedSyncRecord>(
            "SELECT id, filename, mime_type, media_type, size_bytes, width, height, \
             duration_secs, taken_at, created_at, encrypted_blob_id, encrypted_thumb_blob_id, \
             is_favorite, crop_metadata, photo_hash, source_path, \
             photo_subtype, burst_id, motion_video_blob_id \
             FROM photos \
             WHERE user_id = ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
             AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
             AND (encrypted_blob_id IS NULL OR encrypted_blob_id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL)) \
             AND (COALESCE(taken_at, created_at) < ? \
                  OR (COALESCE(taken_at, created_at) = ? AND id > ?)) \
             ORDER BY COALESCE(taken_at, created_at) DESC, id ASC \
             LIMIT ?",
        )
        .bind(user_id)
        .bind(&cursor_ts)
        .bind(&cursor_ts)
        .bind(&cursor_id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, EncryptedSyncRecord>(
            "SELECT id, filename, mime_type, media_type, size_bytes, width, height, \
             duration_secs, taken_at, created_at, encrypted_blob_id, encrypted_thumb_blob_id, \
             is_favorite, crop_metadata, photo_hash, source_path, \
             photo_subtype, burst_id, motion_video_blob_id \
             FROM photos \
             WHERE user_id = ? \
             AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
             AND id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
             AND (encrypted_blob_id IS NULL OR encrypted_blob_id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL)) \
             ORDER BY COALESCE(taken_at, created_at) DESC, id ASC \
             LIMIT ?",
        )
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

    Ok(EncryptedSyncResponse {
        photos,
        next_cursor,
    })
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
