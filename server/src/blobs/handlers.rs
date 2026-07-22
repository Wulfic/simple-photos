//! Encrypted blob storage endpoints.
//!
//! Handles upload (with SHA-256 integrity check and per-user quota
//! enforcement), paginated listing, and deletion with audit logging.
//! Streaming download lives in [`super::download`].

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

use super::models::*;
use super::storage;

/// All valid blob types.  The server treats blobs as opaque encrypted bytes —
/// the type is stored as metadata only, for client-side querying.
const VALID_BLOB_TYPES: &[&str] = &[
    "photo",
    "gif",
    "video",
    "audio",
    "thumbnail",
    "video_thumbnail",
    "album_manifest",
];

/// Query parameters for the blob list endpoint.
#[derive(Debug, Deserialize)]
pub struct ListBlobsQuery {
    /// Filter by blob type (e.g. "photo", "video", "thumbnail").
    pub blob_type: Option<String>,
    /// Cursor for pagination — `upload_time` of the last item from the previous page.
    pub after: Option<String>,
    /// Maximum items to return (default 50, max 200).
    pub limit: Option<i64>,
}

/// Parse the optional `X-Blob-Format` header into a `blobs.blob_format` value.
///
/// Clients that produce the legacy v1 monolithic envelope omit the header, so
/// the default is `1`. Streaming clients that upload a v2 chunked container
/// (see [`super::chunked`]) send `X-Blob-Format: 2`. Any other value is a
/// client bug — reject it with 400 rather than silently persisting a format the
/// download path and clients can't describe or decrypt.
fn parse_blob_format(headers: &HeaderMap) -> Result<i64, AppError> {
    // Absent header → legacy v1 envelope (the column default), matching every
    // existing client that predates the chunked path.
    let raw = match headers.get("x-blob-format").and_then(|v| v.to_str().ok()) {
        None => return Ok(1),
        Some(s) => s,
    };
    match raw.trim().parse::<i64>() {
        Ok(1) => Ok(1),
        Ok(v) if v == super::chunked::FORMAT_V2 => Ok(super::chunked::FORMAT_V2),
        _ => Err(AppError::BadRequest(format!(
            "Invalid X-Blob-Format '{raw}'. Valid values: 1 (monolithic envelope), 2 (chunked)"
        ))),
    }
}

/// POST /api/blobs — upload an encrypted blob.
///
/// Headers:
/// - `x-blob-type` — one of: photo, gif, video, audio, thumbnail,
///   video_thumbnail, album_manifest (default: "photo")
/// - `x-blob-format` — container format: `1` (legacy monolithic envelope,
///   the default when absent) or `2` (chunked streaming container)
/// - `x-client-hash` — optional SHA-256 hex digest for integrity verification
/// - `x-content-hash` — optional short hash of the *original* (pre-encryption)
///   content, used for cross-platform photo alignment
///
/// Enforces per-user storage quota. Returns 201 with the new blob ID.
///
/// **Streaming:** The request body is streamed directly to disk in chunks
/// while simultaneously computing the SHA-256 hash.  This avoids buffering
/// multi-gigabyte video blobs entirely in server RAM.
pub async fn upload(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<BlobUploadResponse>), AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let blob_type = headers
        .get("x-blob-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("photo")
        .to_string();

    tracing::info!(
        user_id = %auth.user_id,
        blob_type = %blob_type,
        "Blob upload started (streaming)"
    );

    // Validate blob type against allowlist
    if !VALID_BLOB_TYPES.contains(&blob_type.as_str()) {
        tracing::warn!(
            user_id = %auth.user_id,
            blob_type = %blob_type,
            "Blob upload rejected: invalid blob type"
        );
        return Err(AppError::BadRequest(format!(
            "Invalid blob type '{}'. Valid types: {}",
            blob_type,
            VALID_BLOB_TYPES.join(", ")
        )));
    }

    // Container format (1 = legacy monolithic envelope, 2 = chunked). Reject a
    // malformed value before streaming the body to disk.
    let blob_format = parse_blob_format(&headers)?;

    let client_hash = headers
        .get("x-client-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // X-Content-Hash: short hash of the ORIGINAL (pre-encryption) content.
    // Used for cross-platform photo alignment — same original photo always
    // produces the same content_hash regardless of encryption nonce.
    let content_hash = headers
        .get("x-content-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // ── Pre-flight quota check (fast reject using Content-Length header) ─────
    // This avoids streaming the entire body only to reject it at the end.
    // The final size is re-verified after streaming completes.
    let used: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM blobs WHERE user_id = ?")
            .bind(&auth.user_id)
            .fetch_one(&state.read_pool)
            .await?;

    let quota: i64 = sqlx::query_scalar("SELECT storage_quota_bytes FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.read_pool)
        .await?;

    if let Some(cl) = headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
    {
        if cl > state.config.storage.max_blob_size_bytes as i64 {
            return Err(AppError::PayloadTooLarge);
        }
        if quota > 0 && used + cl > quota {
            return Err(AppError::Forbidden("Storage quota exceeded".into()));
        }
    }

    // ── Stream body to disk, computing SHA-256 incrementally ────────────────
    let blob_id = Uuid::new_v4().to_string();
    let storage_root = (**state.storage_root.load()).clone();
    let (storage_path, actual_size, computed_hash) =
        storage::write_blob_streaming(&storage_root, &auth.user_id, &blob_id, body).await?;

    // ── Post-stream validation ──────────────────────────────────────────────
    let cleanup = || async {
        if let Err(e) = storage::delete_blob(&storage_root, &storage_path).await {
            tracing::warn!("Failed to clean up blob at {}: {}", storage_path, e);
        }
    };

    if actual_size == 0 {
        cleanup().await;
        return Err(AppError::BadRequest("Empty blob body".into()));
    }

    if actual_size as i64 > state.config.storage.max_blob_size_bytes as i64 {
        cleanup().await;
        return Err(AppError::PayloadTooLarge);
    }

    // Final quota check with actual streamed size
    if quota > 0 && used + actual_size as i64 > quota {
        cleanup().await;
        return Err(AppError::Forbidden("Storage quota exceeded".into()));
    }

    // Server-side integrity check — compare streamed SHA-256 against client hash
    if let Some(ref expected_hash) = client_hash {
        if computed_hash != *expected_hash {
            tracing::warn!(
                user_id = auth.user_id,
                expected = expected_hash,
                computed = computed_hash,
                "Blob integrity check failed — hash mismatch"
            );
            cleanup().await;
            return Err(AppError::BadRequest(
                "Blob integrity check failed: X-Client-Hash does not match uploaded data".into(),
            ));
        }
    }

    // ── Content-hash dedup ──────────────────────────────────────────────────
    // If the caller provided X-Content-Hash (short hash of the *original*
    // unencrypted data), check whether this user already has a blob with the
    // same content_hash.  Return the existing blob instead of storing a
    // duplicate, mirroring the photo upload dedup behaviour.
    if let Some(ref ch) = content_hash {
        let existing: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT id, upload_time, size_bytes FROM blobs \
             WHERE user_id = ? AND content_hash = ? LIMIT 1",
        )
        .bind(&auth.user_id)
        .bind(ch)
        .fetch_optional(&state.read_pool)
        .await?;

        if let Some((eid, etime, esize)) = existing {
            tracing::info!(
                user_id = %auth.user_id,
                existing_blob_id = %eid,
                content_hash = %ch,
                "Duplicate blob upload detected (content_hash match) — returning existing record"
            );
            // Clean up the file we just wrote — it's a duplicate
            cleanup().await;
            return Ok((
                StatusCode::OK,
                Json(BlobUploadResponse {
                    blob_id: eid,
                    upload_time: etime,
                    size: esize,
                }),
            ));
        }
    }

    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash, blob_format) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .bind(&blob_type)
    .bind(actual_size as i64)
    .bind(&client_hash)
    .bind(&now)
    .bind(&storage_path)
    .bind(&content_hash)
    .bind(blob_format)
    .execute(&state.pool)
    .await?;

    audit::log(
        &state,
        AuditEvent::BlobUpload,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "blob_id": blob_id,
            "blob_type": blob_type,
            "size_bytes": actual_size
        })),
    )
    .await;

    tracing::info!(
        user_id = %auth.user_id,
        blob_id = %blob_id,
        blob_type = %blob_type,
        blob_format = blob_format,
        size_bytes = actual_size,
        "Blob upload completed successfully"
    );

    Ok((
        StatusCode::CREATED,
        Json(BlobUploadResponse {
            blob_id,
            upload_time: now,
            size: actual_size as i64,
        }),
    ))
}

/// Excludes blobs that back a secure-gallery item (directly, via the photo's
/// encrypted/thumb blob, or via an `encrypted_gallery_items` locator). No bind
/// parameters — every subquery is self-contained — so it can be concatenated
/// into any of the `list_blobs_page` variants without shifting the bind order.
const BLOB_SECURE_EXCLUSION: &str = " \
     AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
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
              OR p.id IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))) \
     AND id NOT IN (SELECT encrypted_blob_id FROM encrypted_gallery_items WHERE encrypted_blob_id IS NOT NULL) \
     AND id NOT IN (SELECT encrypted_thumb_blob_id FROM encrypted_gallery_items WHERE encrypted_thumb_blob_id IS NOT NULL)";

/// GET /api/blobs — list blobs for the authenticated user with cursor-based pagination.
/// Supports filtering by `blob_type` and forward-only cursor via `after`.
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ListBlobsQuery>,
) -> Result<Json<BlobListResponse>, AppError> {
    let limit = params.limit.unwrap_or(50).min(200);

    if let Some(ref blob_type) = params.blob_type {
        if !VALID_BLOB_TYPES.contains(&blob_type.as_str()) {
            return Err(AppError::BadRequest(format!(
                "Invalid blob_type filter '{}'. Valid: {}",
                blob_type,
                VALID_BLOB_TYPES.join(", ")
            )));
        }
    }

    Ok(Json(
        list_blobs_page(
            &state.read_pool,
            &auth.user_id,
            params.blob_type.as_deref(),
            params.after.as_deref(),
            limit,
        )
        .await?,
    ))
}

/// Split an `"<upload_time>|<id>"` blob cursor. A legacy bare-timestamp cursor
/// maps to an empty id; because the boundary compares `id > cursor_id` and every
/// real id sorts after the empty string, that re-serves the whole timestamp
/// group — a duplicate, never a skip. Neither an ISO timestamp nor a UUID
/// contains `|`, so splitting on the last `|` is unambiguous.
fn parse_blob_cursor(after: &str) -> (String, String) {
    match after.rfind('|') {
        Some(idx) => (after[..idx].to_string(), after[idx + 1..].to_string()),
        None => (after.to_string(), String::new()),
    }
}

/// Fetch one keyset page of the blob listing.
///
/// Split out of the handler so the pagination contract is unit-testable without
/// an HTTP stack. The cursor is composite — `"<upload_time>|<id>"` — because
/// `upload_time` is **not unique**: batch encryption stamps a run of blobs with
/// the same `upload_time`, so a bare-timestamp cursor with a strict
/// `upload_time > ?` predicate drops every member of such a run after the first
/// whenever a page boundary falls inside it. Same off-by-one as
/// `gallery::sync::fetch_page`, in a paginator that never had an `id` tiebreak.
pub async fn list_blobs_page(
    pool: &sqlx::SqlitePool,
    user_id: &str,
    blob_type: Option<&str>,
    after: Option<&str>,
    limit: i64,
) -> Result<BlobListResponse, AppError> {
    let mut sql = String::from(
        "SELECT id, blob_type, size_bytes, client_hash, upload_time, content_hash FROM blobs \
         WHERE user_id = ?",
    );
    if blob_type.is_some() {
        sql.push_str(" AND blob_type = ?");
    }
    if after.is_some() {
        sql.push_str(" AND (upload_time > ? OR (upload_time = ? AND id > ?))");
    }
    sql.push_str(BLOB_SECURE_EXCLUSION);
    sql.push_str(" ORDER BY upload_time ASC, id ASC LIMIT ?");

    let mut q = sqlx::query_as::<_, BlobRecord>(&sql).bind(user_id.to_string());
    if let Some(bt) = blob_type {
        q = q.bind(bt.to_string());
    }
    if let Some(after) = after {
        let (ts, id) = parse_blob_cursor(after);
        q = q.bind(ts.clone()).bind(ts).bind(id);
    }
    let mut blobs = q.bind(limit + 1).fetch_all(pool).await?;

    // Truncate before deriving the cursor: the extra row exists only to detect
    // a next page, and the next-page predicate is strict, so a cursor built from
    // the peeked row skips it permanently. See `gallery::sync::fetch_page`.
    let has_more = blobs.len() as i64 > limit;
    blobs.truncate(limit as usize);

    let next_cursor = if has_more {
        blobs.last().map(|b| format!("{}|{}", b.upload_time, b.id))
    } else {
        None
    };

    Ok(BlobListResponse { blobs, next_cursor })
}

/// DELETE /api/blobs/:id — delete a blob and its on-disk file. Returns 204 on success.
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(blob_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    // Validate blob_id format
    if Uuid::parse_str(&blob_id).is_err() {
        return Err(AppError::BadRequest("Invalid blob ID format".into()));
    }

    let storage_path = sqlx::query_scalar::<_, String>(
        "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    // Lock-free read via ArcSwap.
    let storage_root = (**state.storage_root.load()).clone();
    storage::delete_blob(&storage_root, &storage_path).await?;

    // Wrap DB operations in a transaction for atomicity
    let mut tx = state.pool.begin().await?;

    sqlx::query("DELETE FROM blobs WHERE id = ? AND user_id = ?")
        .bind(&blob_id)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;

    // Clean up shared album references to prevent dangling photo_ref entries
    sqlx::query("DELETE FROM shared_album_photos WHERE photo_ref = ? AND ref_type = 'blob'")
        .bind(&blob_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    audit::log(
        &state,
        AuditEvent::BlobDelete,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({ "blob_id": blob_id })),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with_format(value: &'static str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-blob-format", HeaderValue::from_static(value));
        h
    }

    #[test]
    fn blob_format_defaults_to_v1_when_header_absent() {
        // Every pre-chunked client omits the header — it must map to v1, the
        // column default, not error.
        assert_eq!(parse_blob_format(&HeaderMap::new()).unwrap(), 1);
    }

    #[test]
    fn blob_format_accepts_explicit_v1() {
        assert_eq!(parse_blob_format(&headers_with_format("1")).unwrap(), 1);
    }

    #[test]
    fn blob_format_accepts_v2() {
        assert_eq!(
            parse_blob_format(&headers_with_format("2")).unwrap(),
            super::super::chunked::FORMAT_V2
        );
    }

    #[test]
    fn blob_format_tolerates_surrounding_whitespace() {
        assert_eq!(parse_blob_format(&headers_with_format(" 2 ")).unwrap(), 2);
    }

    #[test]
    fn blob_format_rejects_unknown_numeric_value() {
        // A format the download path can't describe and clients can't decrypt is
        // a client bug — reject it rather than silently storing it.
        let err = parse_blob_format(&headers_with_format("3")).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn blob_format_rejects_non_numeric_value() {
        let err = parse_blob_format(&headers_with_format("v2")).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn blob_format_rejects_empty_value() {
        let err = parse_blob_format(&headers_with_format("")).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    // ── Keyset pagination (composite upload_time|id cursor) ────────────────

    use std::collections::HashSet;
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

    async fn insert_blob(pool: &sqlx::SqlitePool, id: &str, user: &str, upload_time: &str) {
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
             VALUES (?, ?, 'photo', 0, ?, ?)",
        )
        .bind(id)
        .bind(user)
        .bind(upload_time)
        .bind(format!("blobs/{id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn paginate_all(pool: &sqlx::SqlitePool, user: &str, limit: i64) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let page = list_blobs_page(pool, user, None, cursor.as_deref(), limit)
                .await
                .unwrap();
            ids.extend(page.blobs.iter().map(|b| b.id.clone()));
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
        assert!(missing.is_empty(), "blobs never returned by ANY page: {missing:?}");
        assert_eq!(got.len(), seeded.len(), "expected each blob exactly once");
        assert_eq!(got_set, want_set);
    }

    #[test]
    fn parses_legacy_and_composite_blob_cursors() {
        assert_eq!(
            parse_blob_cursor("2026-01-01T00:00:00Z|b7"),
            ("2026-01-01T00:00:00Z".to_string(), "b7".to_string())
        );
        assert_eq!(
            parse_blob_cursor("2026-01-01T00:00:00Z"),
            ("2026-01-01T00:00:00Z".to_string(), String::new())
        );
    }

    #[tokio::test]
    async fn distinct_upload_times_round_trip() {
        let pool = test_pool().await;
        let mut seeded = Vec::new();
        for i in 0..7 {
            let id = format!("d{i:02}");
            insert_blob(&pool, &id, "u1", &format!("2026-01-{:02}T00:00:00Z", i + 1)).await;
            seeded.push(id);
        }
        for limit in [1, 2, 3, 7] {
            assert_round_trip(&paginate_all(&pool, "u1", limit).await, &seeded);
        }
    }

    /// The bug: batch encryption stamps a run of blobs with one `upload_time`.
    /// A bare-timestamp cursor with a strict `upload_time > ?` drops every blob
    /// in the run after the first at a page boundary. Verified RED against the
    /// old cursor: `limit=1` returned a single blob.
    #[tokio::test]
    async fn blobs_sharing_an_upload_time_survive_a_page_boundary() {
        let pool = test_pool().await;
        let mut seeded = Vec::new();
        for i in 0..5 {
            let id = format!("b{i:02}");
            insert_blob(&pool, &id, "u1", "2026-02-01T00:00:00Z").await;
            seeded.push(id);
        }
        for limit in [1, 2, 3, 4, 5] {
            assert_round_trip(&paginate_all(&pool, "u1", limit).await, &seeded);
        }
    }

    #[tokio::test]
    async fn blob_type_filter_still_paginates_completely() {
        let pool = test_pool().await;
        // Two 'photo' blobs and one 'thumbnail', all sharing one upload_time.
        insert_blob(&pool, "p1", "u1", "2026-03-01T00:00:00Z").await;
        insert_blob(&pool, "p2", "u1", "2026-03-01T00:00:00Z").await;
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
             VALUES ('th1', 'u1', 'thumbnail', 0, '2026-03-01T00:00:00Z', 'blobs/th1')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut got = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let page = list_blobs_page(&pool, "u1", Some("photo"), cursor.as_deref(), 1)
                .await
                .unwrap();
            got.extend(page.blobs.iter().map(|b| b.id.clone()));
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        assert_round_trip(&got, &["p1".to_string(), "p2".to_string()]);
    }
}
