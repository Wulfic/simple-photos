//! Trash read endpoints: list and thumbnail serving.
//!
//! Mutation operations (soft-delete, restore, permanent-delete, empty)
//! live in [`super::operations`].

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::*;

// ── Trash Endpoints ───────────────────────────────────────────────────────────

/// GET /api/trash
/// List all items in the authenticated user's trash.
pub async fn list_trash(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<TrashListQuery>,
) -> Result<Json<TrashListResponse>, AppError> {
    let limit = params.limit.unwrap_or(100).min(500);
    Ok(Json(
        list_trash_page(
            &state.read_pool,
            &auth.user_id,
            params.after.as_deref(),
            limit,
        )
        .await?,
    ))
}

/// Split a `"<deleted_at>|<id>"` trash cursor into its parts.
///
/// A legacy cursor (bare `deleted_at`, no `|`) maps to an empty id. Because the
/// boundary predicate compares `id > cursor_id` and every real id sorts after
/// the empty string, that re-serves the whole timestamp group at the boundary —
/// a duplicate, never a skip. `id` is a UUID and `deleted_at` an ISO timestamp,
/// so neither contains `|`; splitting on the last `|` is unambiguous.
fn parse_trash_cursor(after: &str) -> (String, String) {
    match after.rfind('|') {
        Some(idx) => (after[..idx].to_string(), after[idx + 1..].to_string()),
        None => (after.to_string(), String::new()),
    }
}

/// Fetch one keyset page of the trash listing.
///
/// Split out of the handler so the pagination contract is unit-testable without
/// an HTTP stack. The cursor is composite — `"<deleted_at>|<id>"` — because
/// `deleted_at` is **not unique**: emptying the trash or bulk-deleting stamps
/// every affected row with the same `deleted_at`, so a bare-timestamp cursor
/// with a strict `deleted_at < ?` predicate drops every member of such a group
/// after the first whenever a page boundary falls inside it. This is the same
/// off-by-one `gallery::sync::fetch_page` fixed for the encrypted feed.
pub async fn list_trash_page(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    after: Option<&str>,
    limit: i64,
) -> Result<TrashListResponse, AppError> {
    const COLUMNS: &str = "id, photo_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path, \
         deleted_at, expires_at, encrypted_blob_id, thumbnail_blob_id";

    let mut items = if let Some(after) = after {
        let (cursor_ts, cursor_id) = parse_trash_cursor(after);
        sqlx::query_as::<_, TrashItem>(&format!(
            "SELECT {COLUMNS} FROM trash_items \
             WHERE user_id = ? \
             AND (deleted_at < ? OR (deleted_at = ? AND id > ?)) \
             ORDER BY deleted_at DESC, id ASC LIMIT ?",
        ))
        .bind(user_id)
        .bind(&cursor_ts)
        .bind(&cursor_ts)
        .bind(&cursor_id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, TrashItem>(&format!(
            "SELECT {COLUMNS} FROM trash_items \
             WHERE user_id = ? \
             ORDER BY deleted_at DESC, id ASC LIMIT ?",
        ))
        .bind(user_id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?
    };

    // Truncate before deriving the cursor: the extra row exists only to detect
    // a next page, and the next-page predicate is strict, so a cursor built from
    // the peeked row skips it permanently. See `gallery::sync::fetch_page`.
    let has_more = items.len() as i64 > limit;
    items.truncate(limit as usize);

    let next_cursor = if has_more {
        items.last().map(|i| format!("{}|{}", i.deleted_at, i.id))
    } else {
        None
    };

    Ok(TrashListResponse { items, next_cursor })
}

/// GET /api/trash/:id/thumb
/// Serve the thumbnail for a trashed photo (so users can see what they're restoring).
pub async fn serve_trash_thumbnail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(trash_id): Path<String>,
) -> Result<Response, AppError> {
    let thumb_path: Option<String> =
        sqlx::query_scalar("SELECT thumb_path FROM trash_items WHERE id = ? AND user_id = ?")
            .bind(&trash_id)
            .bind(&auth.user_id)
            .fetch_optional(&state.read_pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let thumb_path = thumb_path.ok_or(AppError::NotFound)?;
    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    let full_path = storage_root.join(&thumb_path);

    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err(AppError::NotFound);
    }

    let meta = tokio::fs::metadata(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read thumbnail: {e}")))?;
    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to open thumbnail: {e}")))?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", HeaderValue::from_static("image/jpeg"))
        .header("Content-Length", HeaderValue::from(meta.len()))
        .header(
            "Cache-Control",
            HeaderValue::from_static("private, max-age=86400"),
        )
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::str::FromStr;

    // foreign_keys(false): we insert bare trash rows without the users graph.
    // See `gallery::sync` tests for the same setup and rationale.
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

    async fn insert(pool: &sqlx::SqlitePool, id: &str, user: &str, deleted_at: &str) {
        sqlx::query(
            "INSERT INTO trash_items (id, user_id, photo_id, filename, file_path, mime_type, \
             media_type, size_bytes, width, height, deleted_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, 'image/jpeg', 'photo', 0, 0, 0, ?, ?)",
        )
        .bind(id)
        .bind(user)
        .bind(format!("photo-{id}"))
        .bind(format!("{id}.jpg"))
        .bind(format!("uploads/{id}.jpg"))
        .bind(deleted_at)
        .bind("2026-12-31T00:00:00Z")
        .execute(pool)
        .await
        .unwrap();
    }

    async fn paginate_all(pool: &sqlx::SqlitePool, user: &str, limit: i64) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let page = list_trash_page(pool, user, cursor.as_deref(), limit)
                .await
                .unwrap();
            ids.extend(page.items.iter().map(|i| i.id.clone()));
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
            "rows never returned by ANY page: {missing:?}"
        );
        assert_eq!(got.len(), seeded.len(), "expected each row exactly once");
        assert_eq!(got_set, want_set);
    }

    #[test]
    fn parses_legacy_and_composite_cursors() {
        assert_eq!(
            parse_trash_cursor("2026-01-01T00:00:00Z|abc"),
            ("2026-01-01T00:00:00Z".to_string(), "abc".to_string())
        );
        // Legacy bare-timestamp cursor => empty id, which re-serves the group.
        assert_eq!(
            parse_trash_cursor("2026-01-01T00:00:00Z"),
            ("2026-01-01T00:00:00Z".to_string(), String::new())
        );
    }

    /// Distinct timestamps: ordinary keyset pagination, every row once.
    #[tokio::test]
    async fn distinct_timestamps_round_trip() {
        let pool = test_pool().await;
        let mut seeded = Vec::new();
        for i in 0..7 {
            let id = format!("d{i:02}");
            insert(&pool, &id, "u1", &format!("2026-01-{:02}T00:00:00Z", i + 1)).await;
            seeded.push(id);
        }
        for limit in [1, 2, 3, 7] {
            assert_round_trip(&paginate_all(&pool, "u1", limit).await, &seeded);
        }
    }

    /// The bug: emptying the trash stamps every row with one `deleted_at`. A
    /// bare-timestamp cursor with a strict `deleted_at < ?` drops the group after
    /// the first at any page boundary inside it. Verified RED against the old
    /// timestamp-only cursor: `limit=1` returned a single row.
    #[tokio::test]
    async fn rows_sharing_a_deleted_at_survive_a_page_boundary() {
        let pool = test_pool().await;
        let mut seeded = Vec::new();
        for i in 0..5 {
            let id = format!("t{i:02}");
            insert(&pool, &id, "u1", "2026-02-01T00:00:00Z").await;
            seeded.push(id);
        }
        for limit in [1, 2, 3, 4, 5] {
            assert_round_trip(&paginate_all(&pool, "u1", limit).await, &seeded);
        }
    }

    #[tokio::test]
    async fn pagination_is_scoped_to_the_user() {
        let pool = test_pool().await;
        insert(&pool, "mine-1", "u1", "2026-03-01T00:00:00Z").await;
        insert(&pool, "mine-2", "u1", "2026-03-01T00:00:00Z").await;
        insert(&pool, "theirs", "u2", "2026-03-01T00:00:00Z").await;
        assert_round_trip(
            &paginate_all(&pool, "u1", 1).await,
            &["mine-1".to_string(), "mine-2".to_string()],
        );
    }
}
