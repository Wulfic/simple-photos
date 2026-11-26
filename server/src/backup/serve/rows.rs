//! Row-only receive endpoints (TODO.md F9).
//!
//! `POST /api/backup/receive-photo-rows` and `POST /api/backup/receive-trash-rows`
//! take JSON batches of rows whose primary-side `file_path` is `''` — Android
//! uploads (`register-encrypted`) and encrypted Save Copies. Their bytes reach the
//! backup as blobs (sync phase 3); before F9 the rows that index those blobs never
//! did, because the sync engine could only send a row as a file.
//!
//! Why not let `/backup/receive` accept an empty path: that handler is built around
//! a file, and its trash branch deletes `photos` rows `WHERE … OR file_path = ?` —
//! with `''` that matches every encrypted-only row on the backup. Every write here
//! is keyed on a non-empty `id` only, and the stored `file_path` is always `''`.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;

use crate::error::AppError;
use crate::photos::utils::normalize_iso_timestamp;
use crate::state::AppState;

use super::super::models::{
    EncryptedPhotoRow, EncryptedTrashRow, RowBatch, RowBatchResult, RowFailure, MAX_ROWS_PER_BATCH,
};
use super::super::serve_receive::resolve_backup_owner;
use super::validate_api_key;

/// POST /api/backup/receive-photo-rows
pub async fn backup_receive_photo_rows(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(batch): Json<RowBatch<EncryptedPhotoRow>>,
) -> Result<Json<RowBatchResult>, AppError> {
    validate_api_key(&state, &headers).await?;
    check_batch_size(batch.rows.len())?;
    let result = receive_photo_rows(&state.pool, &batch.rows).await;
    tracing::info!(
        accepted = result.accepted.len(),
        failed = result.failed.len(),
        "Backup received encrypted-only photo rows"
    );
    Ok(Json(result))
}

/// POST /api/backup/receive-trash-rows
pub async fn backup_receive_trash_rows(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(batch): Json<RowBatch<EncryptedTrashRow>>,
) -> Result<Json<RowBatchResult>, AppError> {
    validate_api_key(&state, &headers).await?;
    check_batch_size(batch.rows.len())?;
    let result = receive_trash_rows(&state.pool, &batch.rows).await;
    tracing::info!(
        accepted = result.accepted.len(),
        failed = result.failed.len(),
        "Backup received encrypted-only trash rows"
    );
    Ok(Json(result))
}

fn check_batch_size(n: usize) -> Result<(), AppError> {
    if n > MAX_ROWS_PER_BATCH {
        return Err(AppError::BadRequest(format!(
            "{n} rows in one batch; the limit is {MAX_ROWS_PER_BATCH}"
        )));
    }
    Ok(())
}

pub(crate) async fn receive_photo_rows(
    pool: &sqlx::SqlitePool,
    rows: &[EncryptedPhotoRow],
) -> RowBatchResult {
    let mut result = RowBatchResult::default();
    for row in rows {
        match upsert_photo_row(pool, row).await {
            Ok(()) => result.accepted.push(row.id.clone()),
            Err(e) => {
                tracing::warn!(photo_id = %row.id, error = %e, "Backup: photo row rejected");
                result.failed.push(RowFailure {
                    id: row.id.clone(),
                    error: e,
                });
            }
        }
    }
    result
}

pub(crate) async fn receive_trash_rows(
    pool: &sqlx::SqlitePool,
    rows: &[EncryptedTrashRow],
) -> RowBatchResult {
    let mut result = RowBatchResult::default();
    for row in rows {
        match apply_trash_row(pool, row).await {
            Ok(()) => result.accepted.push(row.id.clone()),
            Err(e) => {
                tracing::warn!(trash_id = %row.id, error = %e, "Backup: trash row rejected");
                result.failed.push(RowFailure {
                    id: row.id.clone(),
                    error: e,
                });
            }
        }
    }
    result
}

/// One photo row, in one transaction. The upsert mirrors `/backup/receive`
/// (same COALESCE rules) except that `file_path` is always `''` and no
/// thumbnail path is invented. A hash already held by a DIFFERENT id on this
/// backup trips `idx_photos_user_hash` and fails this row — nothing is deleted
/// to make room.
async fn upsert_photo_row(pool: &sqlx::SqlitePool, row: &EncryptedPhotoRow) -> Result<(), String> {
    if row.id.trim().is_empty() {
        return Err("refused: empty photo id".into());
    }
    let owner = resolve_backup_owner(pool, Some(&row.user_id))
        .await
        .map_err(|e| format!("owner: {e}"))?;
    let created_at = normalize_iso_timestamp(&row.created_at);

    let mut tx = crate::db::begin_write(pool)
        .await
        .map_err(|e| format!("begin: {e}"))?;

    sqlx::query(
        "INSERT INTO photos (
            id, user_id, filename, file_path, mime_type, media_type,
            size_bytes, width, height, taken_at, latitude, longitude,
            duration_secs, camera_model, is_favorite, photo_hash, crop_metadata,
            thumb_path, created_at, encrypted_blob_id, encrypted_thumb_blob_id
        ) VALUES (?, ?, ?, '', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            file_path     = excluded.file_path,
            size_bytes    = excluded.size_bytes,
            taken_at      = COALESCE(excluded.taken_at,    taken_at),
            latitude      = COALESCE(excluded.latitude,    latitude),
            longitude     = COALESCE(excluded.longitude,   longitude),
            width         = CASE WHEN excluded.width  > 0 THEN excluded.width  ELSE width  END,
            height        = CASE WHEN excluded.height > 0 THEN excluded.height ELSE height END,
            duration_secs = COALESCE(excluded.duration_secs,  duration_secs),
            camera_model  = COALESCE(excluded.camera_model,   camera_model),
            is_favorite   = excluded.is_favorite,
            photo_hash    = COALESCE(excluded.photo_hash,     photo_hash),
            crop_metadata = COALESCE(excluded.crop_metadata,  crop_metadata),
            encrypted_blob_id       = COALESCE(excluded.encrypted_blob_id,       encrypted_blob_id),
            encrypted_thumb_blob_id = COALESCE(excluded.encrypted_thumb_blob_id, encrypted_thumb_blob_id)",
    )
    .bind(&row.id)
    .bind(&owner)
    .bind(&row.filename)
    .bind(&row.mime_type)
    .bind(&row.media_type)
    .bind(row.size_bytes)
    .bind(row.width)
    .bind(row.height)
    .bind(row.taken_at.as_deref().map(normalize_iso_timestamp))
    .bind(row.latitude)
    .bind(row.longitude)
    .bind(row.duration_secs)
    .bind(&row.camera_model)
    .bind(row.is_favorite)
    .bind(&row.photo_hash)
    .bind(&row.crop_metadata)
    .bind(&created_at)
    .bind(&row.encrypted_blob_id)
    .bind(&row.encrypted_thumb_blob_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("upsert photo: {e}"))?;

    for tag in row.tags.iter().filter(|t| !t.is_empty()) {
        sqlx::query(
            "INSERT OR IGNORE INTO photo_tags (photo_id, user_id, tag, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&owner)
        .bind(tag)
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("tag {tag:?}: {e}"))?;
    }

    // Live on the primary ⇒ not in trash there (soft-delete removes the photos
    // row). Drop a stale backup trash row for this photo, by its own ids only.
    sqlx::query("DELETE FROM trash_items WHERE photo_id = ? OR id = ?")
        .bind(&row.id)
        .bind(&row.id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("clear stale trash: {e}"))?;

    tx.commit().await.map_err(|e| format!("commit: {e}"))
}

/// One trash row, in one transaction: upsert it, then remove the trashed
/// photo's live row on this backup — by `photos.id` and nothing else. Blobs
/// are kept: a restore needs them.
async fn apply_trash_row(pool: &sqlx::SqlitePool, row: &EncryptedTrashRow) -> Result<(), String> {
    if row.id.trim().is_empty() {
        return Err("refused: empty trash id".into());
    }
    if row.photo_id.trim().is_empty() {
        return Err("refused: empty photo_id".into());
    }
    let owner = resolve_backup_owner(pool, Some(&row.user_id))
        .await
        .map_err(|e| format!("owner: {e}"))?;

    let mut tx = crate::db::begin_write(pool)
        .await
        .map_err(|e| format!("begin: {e}"))?;

    sqlx::query(
        "INSERT INTO trash_items (
            id, user_id, photo_id, filename, file_path, mime_type, media_type,
            size_bytes, width, height, taken_at, latitude, longitude,
            duration_secs, camera_model, is_favorite, photo_hash, crop_metadata,
            thumb_path, deleted_at, expires_at, encrypted_blob_id, encrypted_thumb_blob_id
        ) VALUES (?, ?, ?, ?, '', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            file_path     = excluded.file_path,
            size_bytes    = excluded.size_bytes,
            taken_at      = COALESCE(excluded.taken_at,     taken_at),
            latitude      = COALESCE(excluded.latitude,     latitude),
            longitude     = COALESCE(excluded.longitude,    longitude),
            width         = CASE WHEN excluded.width  > 0 THEN excluded.width  ELSE width  END,
            height        = CASE WHEN excluded.height > 0 THEN excluded.height ELSE height END,
            duration_secs = COALESCE(excluded.duration_secs,  duration_secs),
            camera_model  = COALESCE(excluded.camera_model,   camera_model),
            is_favorite   = excluded.is_favorite,
            photo_hash    = COALESCE(excluded.photo_hash,     photo_hash),
            crop_metadata = COALESCE(excluded.crop_metadata,  crop_metadata),
            deleted_at    = excluded.deleted_at,
            expires_at    = excluded.expires_at,
            encrypted_blob_id       = COALESCE(excluded.encrypted_blob_id,       encrypted_blob_id),
            encrypted_thumb_blob_id = COALESCE(excluded.encrypted_thumb_blob_id, encrypted_thumb_blob_id)",
    )
    .bind(&row.id)
    .bind(&owner)
    .bind(&row.photo_id)
    .bind(&row.filename)
    .bind(&row.mime_type)
    .bind(&row.media_type)
    .bind(row.size_bytes)
    .bind(row.width)
    .bind(row.height)
    .bind(row.taken_at.as_deref().map(normalize_iso_timestamp))
    .bind(row.latitude)
    .bind(row.longitude)
    .bind(row.duration_secs)
    .bind(&row.camera_model)
    .bind(row.is_favorite)
    .bind(&row.photo_hash)
    .bind(&row.crop_metadata)
    .bind(normalize_iso_timestamp(&row.deleted_at))
    .bind(normalize_iso_timestamp(&row.expires_at))
    .bind(&row.encrypted_blob_id)
    .bind(&row.encrypted_thumb_blob_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("upsert trash: {e}"))?;

    let removed = sqlx::query("DELETE FROM photos WHERE id = ?")
        .bind(&row.photo_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("remove live row: {e}"))?
        .rows_affected();
    sqlx::query("DELETE FROM photo_tags WHERE photo_id = ?")
        .bind(&row.photo_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("remove tags: {e}"))?;

    tx.commit().await.map_err(|e| format!("commit: {e}"))?;
    if removed > 0 {
        tracing::debug!(photo_id = %row.photo_id, trash_id = %row.id, "Backup: trashed photo left the gallery");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    async fn pool() -> sqlx::SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for (id, role) in [("admin1", "admin"), ("u1", "user")] {
            sqlx::query(
                "INSERT INTO users (id, username, password_hash, created_at, role) \
                 VALUES (?, ?, 'x', '2026-01-01T00:00:00Z', ?)",
            )
            .bind(id)
            .bind(id)
            .bind(role)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    fn photo(id: &str, hash: Option<&str>) -> EncryptedPhotoRow {
        EncryptedPhotoRow {
            id: id.into(),
            user_id: "u1".into(),
            filename: format!("{id}.jpg"),
            mime_type: "image/jpeg".into(),
            media_type: "photo".into(),
            size_bytes: 0,
            width: 4032,
            height: 3024,
            duration_secs: None,
            taken_at: Some("2026-04-18T02:16:41.872Z".into()),
            latitude: None,
            longitude: None,
            camera_model: None,
            is_favorite: false,
            photo_hash: hash.map(Into::into),
            crop_metadata: None,
            created_at: "2026-11-25T00:00:00Z".into(),
            encrypted_blob_id: Some(format!("blob-{id}")),
            encrypted_thumb_blob_id: Some(format!("thumb-{id}")),
            tags: vec![],
        }
    }

    fn trash(id: &str, photo_id: &str) -> EncryptedTrashRow {
        EncryptedTrashRow {
            id: id.into(),
            photo_id: photo_id.into(),
            user_id: "u1".into(),
            filename: format!("{photo_id}.jpg"),
            mime_type: "image/jpeg".into(),
            media_type: "photo".into(),
            size_bytes: 0,
            width: 4032,
            height: 3024,
            duration_secs: None,
            taken_at: None,
            latitude: None,
            longitude: None,
            camera_model: None,
            is_favorite: false,
            photo_hash: None,
            crop_metadata: None,
            deleted_at: "2026-11-25T00:00:00Z".into(),
            expires_at: "2026-12-25T00:00:00Z".into(),
            encrypted_blob_id: Some(format!("blob-{photo_id}")),
            encrypted_thumb_blob_id: None,
        }
    }

    async fn photo_ids(pool: &sqlx::SqlitePool) -> Vec<String> {
        sqlx::query_scalar("SELECT id FROM photos ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn photo_row_lands_with_empty_path_and_its_blob_ids() {
        let pool = pool().await;
        let mut row = photo("p1", Some("abc123abc123"));
        row.tags = vec!["beach".into(), String::new()];
        let res = receive_photo_rows(&pool, &[row]).await;
        assert_eq!(res.accepted, vec!["p1"]);
        assert!(res.failed.is_empty());

        let (owner, path, blob, thumb, thumb_path): (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT user_id, file_path, encrypted_blob_id, encrypted_thumb_blob_id, thumb_path \
                 FROM photos WHERE id = 'p1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(owner, "u1");
        assert_eq!(path, "");
        assert_eq!(blob.as_deref(), Some("blob-p1"));
        assert_eq!(thumb.as_deref(), Some("thumb-p1"));
        assert_eq!(
            thumb_path, None,
            "no thumbnail path is invented for a row-only photo"
        );
        let tags: Vec<String> =
            sqlx::query_scalar("SELECT tag FROM photo_tags WHERE photo_id = 'p1'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(tags, vec!["beach"], "empty tags are skipped");
    }

    #[tokio::test]
    async fn resending_a_row_updates_it_in_place() {
        let pool = pool().await;
        receive_photo_rows(&pool, &[photo("p1", Some("h1"))]).await;
        let mut again = photo("p1", Some("h1"));
        again.is_favorite = true;
        let res = receive_photo_rows(&pool, &[again]).await;
        assert_eq!(res.accepted, vec!["p1"]);
        assert_eq!(photo_ids(&pool).await, vec!["p1"]);
        let fav: bool = sqlx::query_scalar("SELECT is_favorite FROM photos WHERE id = 'p1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(fav);
    }

    #[tokio::test]
    async fn unknown_owner_falls_back_to_the_backup_admin() {
        let pool = pool().await;
        let mut row = photo("p1", None);
        row.user_id = "not-synced-yet".into();
        receive_photo_rows(&pool, &[row]).await;
        let owner: String = sqlx::query_scalar("SELECT user_id FROM photos WHERE id = 'p1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(owner, "admin1");
    }

    #[tokio::test]
    async fn empty_photo_id_is_refused_and_writes_nothing() {
        let pool = pool().await;
        let res = receive_photo_rows(&pool, &[photo(" ", None)]).await;
        assert!(res.accepted.is_empty());
        assert_eq!(res.failed.len(), 1);
        assert!(photo_ids(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn hash_held_by_another_id_fails_that_row_alone_and_deletes_nothing() {
        let pool = pool().await;
        receive_photo_rows(&pool, &[photo("old", Some("samehash"))]).await;
        let res = receive_photo_rows(
            &pool,
            &[photo("new", Some("samehash")), photo("other", None)],
        )
        .await;
        assert_eq!(res.accepted, vec!["other"]);
        assert_eq!(res.failed.len(), 1);
        assert_eq!(res.failed[0].id, "new");
        assert_eq!(photo_ids(&pool).await, vec!["old", "other"]);
    }

    #[tokio::test]
    async fn a_live_photo_row_clears_its_stale_backup_trash_row() {
        let pool = pool().await;
        receive_trash_rows(&pool, &[trash("t1", "p1")]).await;
        receive_photo_rows(&pool, &[photo("p1", None)]).await;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM trash_items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0, "restored on the primary ⇒ not in the backup's trash");
    }

    #[tokio::test]
    async fn trash_row_removes_only_its_own_photo_among_empty_path_rows() {
        // The trap F9 avoided: every one of these rows has file_path = ''.
        let pool = pool().await;
        let rows: Vec<_> = ["k1", "k2", "k3", "gone"]
            .iter()
            .map(|i| photo(i, None))
            .collect();
        receive_photo_rows(&pool, &rows).await;
        sqlx::query("INSERT INTO photo_tags (photo_id, user_id, tag, created_at) VALUES ('k1','u1','t','x')")
            .execute(&pool)
            .await
            .unwrap();

        let res = receive_trash_rows(&pool, &[trash("t1", "gone")]).await;
        assert_eq!(res.accepted, vec!["t1"]);
        assert_eq!(photo_ids(&pool).await, vec!["k1", "k2", "k3"]);
        let tags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photo_tags")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tags, 1, "other photos' tags survive");

        let (path, photo_id, blob): (String, String, Option<String>) = sqlx::query_as(
            "SELECT file_path, photo_id, encrypted_blob_id FROM trash_items WHERE id = 't1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(path, "");
        assert_eq!(photo_id, "gone");
        assert_eq!(
            blob.as_deref(),
            Some("blob-gone"),
            "a restore needs the blob id"
        );
    }

    #[tokio::test]
    async fn trash_row_with_empty_photo_id_is_refused_and_deletes_nothing() {
        let pool = pool().await;
        receive_photo_rows(&pool, &[photo("k1", None)]).await;
        let res = receive_trash_rows(&pool, &[trash("t1", ""), trash("", "k1")]).await;
        assert!(res.accepted.is_empty());
        assert_eq!(res.failed.len(), 2);
        assert_eq!(photo_ids(&pool).await, vec!["k1"]);
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM trash_items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn oversized_batch_is_refused() {
        assert!(check_batch_size(MAX_ROWS_PER_BATCH).is_ok());
        assert!(check_batch_size(MAX_ROWS_PER_BATCH + 1).is_err());
    }
}
