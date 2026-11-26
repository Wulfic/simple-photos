//! What a trash call deleted from `photos`, kept so restore can put it back
//! (TODO.md F11, migration 041).
//!
//! Both trash paths DELETE the `photos` row, and `foreign_keys` is ON, so the
//! delete also CASCADEs the row's children. Before this module, `trash_items`
//! held a hand-picked subset of the row's columns and restore rebuilt the row
//! from it — or, on the blob path every client uses, did not rebuild it at all.
//!
//! The capture is generic on purpose: a hand-maintained column list is exactly
//! what drifted. Rows are read with `SELECT *` and encoded by SQLite storage
//! class; restore inserts only the keys that are columns of the table *now*,
//! so a column added after the trash takes its default and a dropped one is
//! skipped.
//!
//! Child tables are classified, and `fk_children_are_all_classified` fails if a
//! migration adds a table with an FK to `photos` that is in neither list:
//!
//! * [`SNAPSHOT_CHILDREN`] — authored by a user or an import; nothing could
//!   recreate them, so they are captured and restored with the row.
//! * [`REGENERATED_CHILDREN`] — derived. Detections may point at clusters that
//!   are gone by the time of a restore, and deleting a rendition row queues its
//!   blob for the orphan sweeper (migration 035), so neither can be put back.
//!   Restore clears `ai_processed_photos` so AI re-runs; the rung backfill
//!   already re-queues any video with no renditions.

use base64::Engine as _;
use serde_json::{Map, Number, Value};
use sqlx::sqlite::{SqliteArguments, SqliteRow};
use sqlx::{Column, Row, Sqlite, SqliteConnection, TypeInfo, ValueRef};

use crate::error::AppError;

/// Child tables captured with the row and re-inserted on restore.
pub const SNAPSHOT_CHILDREN: &[&str] = &["edit_copies", "photo_source_albums"];

/// Child tables the server rebuilds by itself after a restore.
pub const REGENERATED_CHILDREN: &[&str] = &[
    "face_detections",
    "object_detections",
    "pet_detections",
    "video_renditions",
];

/// JSON key marking a BLOB value (base64). TEXT is always a JSON string, so an
/// object can only ever be a BLOB.
const BLOB_KEY: &str = "$b64";

type Query<'q> = sqlx::query::Query<'q, Sqlite, SqliteArguments<'q>>;

/// Identifiers in this module come from the schema, never from a request, but
/// they are spliced into SQL — refuse anything that is not a plain name.
fn ident(name: &str) -> Result<String, AppError> {
    if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        Ok(format!("\"{name}\""))
    } else {
        tracing::error!(identifier = %name, "trash snapshot: refusing SQL identifier");
        Err(AppError::Internal(format!(
            "refusing SQL identifier {name:?}"
        )))
    }
}

/// Encode one row as `{column: value}` by storage class.
fn encode_row(row: &SqliteRow) -> Result<Map<String, Value>, AppError> {
    let mut out = Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let class = {
            let raw = row.try_get_raw(i)?;
            if raw.is_null() {
                "NULL".to_string()
            } else {
                raw.type_info().name().to_string()
            }
        };
        let value = match class.as_str() {
            "NULL" => Value::Null,
            "INTEGER" => Value::from(row.try_get_unchecked::<i64, _>(i)?),
            "REAL" => {
                let f: f64 = row.try_get_unchecked(i)?;
                Value::Number(Number::from_f64(f).ok_or_else(|| {
                    tracing::error!(column = %col.name(), value = f, "trash snapshot: non-finite REAL");
                    AppError::Internal(format!("non-finite REAL in column {}", col.name()))
                })?)
            }
            "TEXT" => Value::String(row.try_get_unchecked::<String, _>(i)?),
            "BLOB" => {
                let bytes: Vec<u8> = row.try_get_unchecked(i)?;
                let mut m = Map::new();
                m.insert(
                    BLOB_KEY.into(),
                    Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)),
                );
                Value::Object(m)
            }
            other => {
                tracing::error!(column = %col.name(), class = %other, "trash snapshot: unknown storage class");
                return Err(AppError::Internal(format!(
                    "unknown storage class {other} in column {}",
                    col.name()
                )));
            }
        };
        out.insert(col.name().to_string(), value);
    }
    Ok(out)
}

fn bind_json<'q>(q: Query<'q>, column: &str, v: &Value) -> Result<Query<'q>, AppError> {
    Ok(match v {
        Value::Null => q.bind(None::<String>),
        Value::Bool(b) => q.bind(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                return Err(AppError::Internal(format!(
                    "unrepresentable number in {column}"
                )));
            }
        }
        Value::String(s) => q.bind(s.clone()),
        Value::Object(m) => {
            let b64 = m.get(BLOB_KEY).and_then(Value::as_str).ok_or_else(|| {
                AppError::Internal(format!("snapshot object in {column} is not a BLOB"))
            })?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| AppError::Internal(format!("bad BLOB in {column}: {e}")))?;
            q.bind(bytes)
        }
        Value::Array(_) => {
            return Err(AppError::Internal(format!("snapshot array in {column}")));
        }
    })
}

async fn current_columns(
    conn: &mut SqliteConnection,
    table: &str,
) -> Result<Vec<String>, AppError> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT name FROM pragma_table_info(?)")
            .bind(table)
            .fetch_all(&mut *conn)
            .await?,
    )
}

/// INSERT `row` into `table`, keeping only keys that are current columns.
async fn insert_row(
    conn: &mut SqliteConnection,
    table: &str,
    columns: &[String],
    row: &Map<String, Value>,
) -> Result<(), AppError> {
    let keep: Vec<&String> = row.keys().filter(|k| columns.contains(k)).collect();
    let names = keep
        .iter()
        .map(|k| ident(k))
        .collect::<Result<Vec<_>, _>>()?;
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        ident(table)?,
        names.join(", "),
        vec!["?"; keep.len()].join(", ")
    );
    let mut q = sqlx::query(&sql);
    for k in keep {
        q = bind_json(q, k, &row[k.as_str()])?;
    }
    q.execute(&mut *conn).await?;
    Ok(())
}

/// Capture each `photos` row in `photo_ids` (and its [`SNAPSHOT_CHILDREN`])
/// under `trash_id`. Call inside the trash transaction, BEFORE the DELETE.
/// Ids with no row are skipped. Returns how many rows were captured.
pub async fn capture(
    conn: &mut SqliteConnection,
    trash_id: &str,
    photo_ids: &[String],
) -> Result<usize, AppError> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut captured = 0;
    for photo_id in photo_ids {
        let Some(row) = sqlx::query("SELECT * FROM photos WHERE id = ?")
            .bind(photo_id)
            .fetch_optional(&mut *conn)
            .await?
        else {
            continue;
        };
        let photo_row = encode_row(&row)?;

        let mut children = Map::new();
        for table in SNAPSHOT_CHILDREN {
            let sql = format!("SELECT * FROM {} WHERE photo_id = ?", ident(table)?);
            let rows = sqlx::query(&sql)
                .bind(photo_id)
                .fetch_all(&mut *conn)
                .await?;
            if !rows.is_empty() {
                let encoded = rows
                    .iter()
                    .map(|r| encode_row(r).map(Value::Object))
                    .collect::<Result<Vec<_>, _>>()?;
                children.insert((*table).to_string(), Value::Array(encoded));
            }
        }

        sqlx::query(
            "INSERT INTO trash_photo_snapshots (trash_id, photo_id, photo_row, children, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(trash_id)
        .bind(photo_id)
        .bind(Value::Object(photo_row).to_string())
        .bind(Value::Object(children).to_string())
        .bind(&now)
        .execute(&mut *conn)
        .await?;
        captured += 1;
    }
    Ok(captured)
}

/// Capture each `blobs` row in `blob_ids` owned by `owner_id` under
/// `trash_id`, verbatim. Call inside the trash transaction, BEFORE the DELETE,
/// with the same owner the DELETE uses. Ids with no such row are skipped: a
/// client-supplied thumbnail id may name nothing — or another user's blob,
/// which the DELETE leaves alone, so capturing it would make restore collide
/// with a live row forever. Returns how many rows were captured.
pub async fn capture_blobs(
    conn: &mut SqliteConnection,
    trash_id: &str,
    owner_id: &str,
    blob_ids: &[&str],
) -> Result<usize, AppError> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut captured = 0;
    for blob_id in blob_ids {
        let Some(row) = sqlx::query("SELECT * FROM blobs WHERE id = ? AND user_id = ?")
            .bind(blob_id)
            .bind(owner_id)
            .fetch_optional(&mut *conn)
            .await?
        else {
            continue;
        };
        sqlx::query(
            "INSERT INTO trash_blob_snapshots (trash_id, blob_id, blob_row, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(trash_id)
        .bind(blob_id)
        .bind(Value::Object(encode_row(&row)?).to_string())
        .bind(&now)
        .execute(&mut *conn)
        .await?;
        captured += 1;
    }
    Ok(captured)
}

/// Re-insert every `blobs` row captured under `trash_id`. Returns the restored
/// blob ids; empty ⇒ the trash row predates migration 041 and the caller must
/// rebuild the rows the legacy way. A blob id that is live again is a 409.
pub async fn restore_blobs(
    conn: &mut SqliteConnection,
    trash_id: &str,
) -> Result<Vec<String>, AppError> {
    let snaps = sqlx::query_as::<_, (String, String)>(
        "SELECT blob_id, blob_row FROM trash_blob_snapshots WHERE trash_id = ? ORDER BY blob_id",
    )
    .bind(trash_id)
    .fetch_all(&mut *conn)
    .await?;
    if snaps.is_empty() {
        return Ok(Vec::new());
    }
    let columns = current_columns(conn, "blobs").await?;
    let mut restored = Vec::with_capacity(snaps.len());
    for (blob_id, json) in snaps {
        let Ok(Value::Object(row)) = serde_json::from_str::<Value>(&json) else {
            tracing::error!(%trash_id, %blob_id, "trash snapshot: unreadable blob_row JSON");
            return Err(AppError::Internal(format!(
                "unreadable blob snapshot for {blob_id}"
            )));
        };
        match insert_row(conn, "blobs", &columns, &row).await {
            Ok(()) => restored.push(blob_id),
            Err(AppError::Sqlx(sqlx::Error::Database(db))) if db.is_unique_violation() => {
                tracing::warn!(%trash_id, %blob_id, error = %db, "trash restore: blob id is live again; nothing restored");
                return Err(AppError::Conflict(
                    "This item's storage is already in use. Nothing was restored.".into(),
                ));
            }
            Err(e) => {
                tracing::error!(%trash_id, %blob_id, error = %e, "trash restore: blob row insert failed");
                return Err(e);
            }
        }
    }
    Ok(restored)
}

/// Re-insert every row captured under `trash_id`. Call inside the restore
/// transaction; the caller deletes the trash row (which drops the snapshot).
///
/// A UNIQUE collision on the photo row (the same content was uploaded again
/// while this copy sat in the trash) is a 409: the caller's transaction rolls
/// back and the trash row and snapshot are untouched. A child row whose own FK
/// target has gone is skipped with a warning — the photo matters more than one
/// membership. Returns the restored photo ids (empty ⇒ no snapshot: legacy).
pub async fn restore(conn: &mut SqliteConnection, trash_id: &str) -> Result<Vec<String>, AppError> {
    let snaps = sqlx::query_as::<_, (String, String, String)>(
        "SELECT photo_id, photo_row, children FROM trash_photo_snapshots \
         WHERE trash_id = ? ORDER BY photo_id",
    )
    .bind(trash_id)
    .fetch_all(&mut *conn)
    .await?;
    if snaps.is_empty() {
        return Ok(Vec::new());
    }

    let photo_columns = current_columns(conn, "photos").await?;
    let mut restored = Vec::with_capacity(snaps.len());
    for (photo_id, photo_json, children_json) in snaps {
        let parse = |s: &str, what: &str| -> Result<Map<String, Value>, AppError> {
            match serde_json::from_str::<Value>(s) {
                Ok(Value::Object(m)) => Ok(m),
                other => {
                    tracing::error!(%trash_id, %photo_id, what, ?other, "trash snapshot: unreadable JSON");
                    Err(AppError::Internal(format!(
                        "unreadable {what} snapshot for {photo_id}"
                    )))
                }
            }
        };
        let photo_row = parse(&photo_json, "photo_row")?;
        let children = parse(&children_json, "children")?;

        match insert_row(conn, "photos", &photo_columns, &photo_row).await {
            Ok(()) => {}
            Err(AppError::Sqlx(sqlx::Error::Database(db))) if db.is_unique_violation() => {
                tracing::warn!(%trash_id, %photo_id, error = %db, "trash restore: photo collides with a live row; nothing restored");
                return Err(AppError::Conflict(
                    "The library already holds this photo (it was added again while this \
                     copy was in the trash). Nothing was restored."
                        .into(),
                ));
            }
            Err(e) => {
                tracing::error!(%trash_id, %photo_id, error = %e, "trash restore: photo row insert failed");
                return Err(e);
            }
        }

        for (table, rows) in &children {
            if REGENERATED_CHILDREN.contains(&table.as_str()) {
                // Captured by an older classification; the server rebuilds it.
                tracing::debug!(%trash_id, %photo_id, %table, "trash restore: child table is regenerated, not restored");
                continue;
            }
            if !SNAPSHOT_CHILDREN.contains(&table.as_str()) {
                tracing::warn!(%trash_id, %photo_id, %table, "trash restore: snapshot names an unclassified child table; skipped");
                continue;
            }
            let columns = current_columns(conn, table).await?;
            for row in rows.as_array().map(Vec::as_slice).unwrap_or_default() {
                let Some(row) = row.as_object() else { continue };
                match insert_row(conn, table, &columns, row).await {
                    Ok(()) => {}
                    Err(AppError::Sqlx(sqlx::Error::Database(db)))
                        if db.is_foreign_key_violation() =>
                    {
                        tracing::warn!(%trash_id, %photo_id, %table, error = %db, "trash restore: child row's FK target is gone; skipped");
                    }
                    Err(AppError::Sqlx(sqlx::Error::Database(db))) if db.is_unique_violation() => {
                        tracing::warn!(%trash_id, %photo_id, %table, error = %db, "trash restore: child row already present; skipped");
                    }
                    Err(e) => {
                        tracing::error!(%trash_id, %photo_id, %table, error = %e, "trash restore: child row insert failed");
                        return Err(e);
                    }
                }
            }
        }

        // Detections were cascaded away; without this the restored photo reads
        // as "AI done" forever and never gets them back.
        sqlx::query("DELETE FROM ai_processed_photos WHERE photo_id = ?")
            .bind(&photo_id)
            .execute(&mut *conn)
            .await?;

        restored.push(photo_id);
    }
    Ok(restored)
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
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, role) \
             VALUES ('u1', 'u1', 'x', '2026-01-01T00:00:00Z', 'user')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    /// A registered (Android-shaped) row: no file, blob ids, a REAL, NULLs.
    async fn seed_photo(pool: &sqlx::SqlitePool, id: &str, hash: &str) {
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, taken_at, latitude, created_at, encrypted_blob_id, \
             encrypted_thumb_blob_id, photo_hash, photo_subtype) \
             VALUES (?, 'u1', 'PXL.jpg', '', 'image/jpeg', 'photo', 8192, 4032, 3024, \
             '2026-04-18T02:16:41.872Z', 3.0, '2026-04-18T02:16:41.872Z', ?, ?, ?, 'burst')",
        )
        .bind(id)
        .bind(format!("blob-{id}"))
        .bind(format!("thumb-{id}"))
        .bind(hash)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_trash(pool: &sqlx::SqlitePool, trash_id: &str, photo_id: &str) {
        sqlx::query(
            "INSERT INTO trash_items (id, user_id, photo_id, filename, file_path, mime_type, \
             deleted_at, expires_at) \
             VALUES (?, 'u1', ?, 'PXL.jpg', '', 'image/jpeg', '2026-11-25', '2026-12-25')",
        )
        .bind(trash_id)
        .bind(photo_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn photo_json(pool: &sqlx::SqlitePool, id: &str) -> Option<Map<String, Value>> {
        sqlx::query("SELECT * FROM photos WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .unwrap()
            .map(|r| encode_row(&r).unwrap())
    }

    async fn count(pool: &sqlx::SqlitePool, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    /// Trash the way the handlers do: capture, then DELETE (which cascades).
    async fn trash_photo(pool: &sqlx::SqlitePool, trash_id: &str, photo_id: &str) {
        seed_trash(pool, trash_id, photo_id).await;
        let mut tx = crate::db::begin_write(pool).await.unwrap();
        let n = capture(&mut tx, trash_id, &[photo_id.to_string()])
            .await
            .unwrap();
        assert_eq!(n, 1);
        sqlx::query("DELETE FROM photos WHERE id = ?")
            .bind(photo_id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn fk_children_are_all_classified() {
        let pool = pool().await;
        let actual: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT m.name FROM sqlite_master m \
             JOIN pragma_foreign_key_list(m.name) f \
             WHERE m.type = 'table' AND f.\"table\" = 'photos' ORDER BY m.name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let mut classified: Vec<String> = SNAPSHOT_CHILDREN
            .iter()
            .chain(REGENERATED_CHILDREN)
            .map(|s| s.to_string())
            .collect();
        classified.sort();
        assert_eq!(
            actual, classified,
            "a table with an FK to photos is cascaded away by every trash call — add it to \
             SNAPSHOT_CHILDREN (restore puts it back) or REGENERATED_CHILDREN (the server \
             rebuilds it), in trash/snapshot.rs"
        );
    }

    #[tokio::test]
    async fn codec_round_trips_every_storage_class() {
        let pool = pool().await;
        let mut conn = pool.acquire().await.unwrap();
        sqlx::query("CREATE TEMP TABLE t (i INTEGER, r REAL, s TEXT, b BLOB, n TEXT)")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query(r#"INSERT INTO t VALUES (3, 3.0, '{"$b64":"x"}', X'00FF10', NULL)"#)
            .execute(&mut *conn)
            .await
            .unwrap();
        let row = sqlx::query("SELECT * FROM t")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        let enc = encode_row(&row).unwrap();
        // Through text, as the snapshot table stores it.
        let enc: Map<String, Value> =
            serde_json::from_str(&Value::Object(enc).to_string()).unwrap();
        sqlx::query("DELETE FROM t")
            .execute(&mut *conn)
            .await
            .unwrap();
        let cols = current_columns(&mut conn, "t").await.unwrap();
        insert_row(&mut conn, "t", &cols, &enc).await.unwrap();

        let (ti, tr, s, b, n): (String, String, String, Vec<u8>, Option<String>) =
            sqlx::query_as("SELECT typeof(i), typeof(r), s, b, n FROM t")
                .fetch_one(&mut *conn)
                .await
                .unwrap();
        assert_eq!((ti.as_str(), tr.as_str()), ("integer", "real"));
        assert_eq!(
            s, r#"{"$b64":"x"}"#,
            "TEXT that looks like a BLOB marker stays TEXT"
        );
        assert_eq!(b, vec![0x00, 0xFF, 0x10]);
        assert_eq!(n, None);
    }

    #[tokio::test]
    async fn restore_puts_back_the_exact_row_and_its_authored_children() {
        let pool = pool().await;
        seed_photo(&pool, "p1", "h1").await;
        for sql in [
            "INSERT INTO edit_copies (id, photo_id, user_id, name, edit_metadata) \
             VALUES ('c1', 'p1', 'u1', 'Cropped', '{}')",
            "INSERT INTO photo_source_albums (photo_id, user_id, album_name, created_at) \
             VALUES ('p1', 'u1', 'Trip', '2026-01-01')",
            "INSERT INTO face_detections (photo_id, user_id, bbox_x, bbox_y, bbox_w, bbox_h, \
             confidence, embedding) VALUES ('p1', 'u1', 0, 0, 1, 1, 0.9, X'01')",
            "INSERT INTO ai_processed_photos (photo_id, user_id, processed_at) \
             VALUES ('p1', 'u1', '2026-01-01')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        let before = photo_json(&pool, "p1").await.unwrap();

        trash_photo(&pool, "t1", "p1").await;
        assert!(photo_json(&pool, "p1").await.is_none());
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM edit_copies").await,
            0,
            "cascade ran"
        );

        let mut tx = crate::db::begin_write(&pool).await.unwrap();
        let ids = restore(&mut tx, "t1").await.unwrap();
        tx.commit().await.unwrap();

        assert_eq!(ids, vec!["p1".to_string()]);
        assert_eq!(
            photo_json(&pool, "p1").await.unwrap(),
            before,
            "every column, every type"
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM edit_copies WHERE id = 'c1' AND name = 'Cropped'"
            )
            .await,
            1
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM photo_source_albums WHERE album_name = 'Trip'"
            )
            .await,
            1
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM face_detections").await,
            0,
            "detections are regenerated, not restored"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM ai_processed_photos").await,
            0,
            "AI must re-run, or the photo never gets its detections back"
        );
    }

    #[tokio::test]
    async fn deleting_the_trash_row_drops_its_snapshot() {
        let pool = pool().await;
        seed_photo(&pool, "p1", "h1").await;
        trash_photo(&pool, "t1", "p1").await;
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM trash_photo_snapshots").await,
            1
        );
        sqlx::query("DELETE FROM trash_items WHERE id = 't1'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM trash_photo_snapshots").await,
            0
        );
    }

    #[tokio::test]
    async fn restore_tolerates_a_column_that_no_longer_exists() {
        let pool = pool().await;
        seed_photo(&pool, "p1", "h1").await;
        trash_photo(&pool, "t1", "p1").await;
        sqlx::query(
            "UPDATE trash_photo_snapshots \
             SET photo_row = json_set(photo_row, '$.dropped_by_a_later_migration', 7)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut tx = crate::db::begin_write(&pool).await.unwrap();
        assert_eq!(
            restore(&mut tx, "t1").await.unwrap(),
            vec!["p1".to_string()]
        );
        tx.commit().await.unwrap();
        assert!(photo_json(&pool, "p1").await.is_some());
    }

    #[tokio::test]
    async fn collision_is_a_conflict_and_changes_nothing() {
        let pool = pool().await;
        seed_photo(&pool, "p1", "h1").await;
        trash_photo(&pool, "t1", "p1").await;
        // The phone re-uploaded the same content while the old copy sat in the trash.
        seed_photo(&pool, "p2", "h1").await;
        let live_before = photo_json(&pool, "p2").await.unwrap();

        {
            let mut tx = crate::db::begin_write(&pool).await.unwrap();
            let err = restore(&mut tx, "t1").await.unwrap_err();
            assert!(matches!(err, AppError::Conflict(_)), "{err:?}");
            // Dropped without commit, as the handler's `?` does.
        }

        assert!(photo_json(&pool, "p1").await.is_none());
        assert_eq!(photo_json(&pool, "p2").await.unwrap(), live_before);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM trash_photo_snapshots").await,
            1
        );
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM trash_items").await, 1);
    }

    #[tokio::test]
    async fn blob_rows_come_back_verbatim() {
        let pool = pool().await;
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, \
             storage_path, content_hash, blob_format) \
             VALUES ('b1', 'u1', 'photo', 8192, 'ch', '2026-04-18T00:00:00Z', 'u1/b1', 'h1', 2)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let row_json = |pool: sqlx::SqlitePool| async move {
            sqlx::query("SELECT * FROM blobs WHERE id = 'b1'")
                .fetch_optional(&pool)
                .await
                .unwrap()
                .map(|r| encode_row(&r).unwrap())
        };
        let before = row_json(pool.clone()).await.unwrap();
        seed_trash(&pool, "t1", "b1").await;

        let mut tx = crate::db::begin_write(&pool).await.unwrap();
        // "nope" is a client-supplied thumbnail id that names nothing.
        assert_eq!(
            capture_blobs(&mut tx, "t1", "u1", &["b1", "nope"])
                .await
                .unwrap(),
            1
        );
        sqlx::query("DELETE FROM blobs WHERE id = 'b1'")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let mut tx = crate::db::begin_write(&pool).await.unwrap();
        assert_eq!(
            restore_blobs(&mut tx, "t1").await.unwrap(),
            vec!["b1".to_string()]
        );
        tx.commit().await.unwrap();
        assert_eq!(
            row_json(pool.clone()).await.unwrap(),
            before,
            "size_bytes is the download's Content-Length; blob_format picks the decoder"
        );
    }

    #[tokio::test]
    async fn another_users_blob_is_never_captured() {
        let pool = pool().await;
        for sql in [
            "INSERT INTO users (id, username, password_hash, created_at, role)              VALUES ('u2', 'u2', 'x', '2026-01-01T00:00:00Z', 'user')",
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path)              VALUES ('theirs', 'u2', 'thumbnail', 512, '2026-04-18T00:00:00Z', 'u2/theirs')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        seed_trash(&pool, "t1", "mine").await;
        let mut tx = crate::db::begin_write(&pool).await.unwrap();
        // A client names someone else's blob as "its" thumbnail.
        assert_eq!(
            capture_blobs(&mut tx, "t1", "u1", &["theirs"])
                .await
                .unwrap(),
            0
        );
        tx.commit().await.unwrap();
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM trash_blob_snapshots").await,
            0,
            "restore would collide with their live row forever"
        );
    }

    #[tokio::test]
    async fn no_snapshot_means_legacy_restore() {
        let pool = pool().await;
        seed_trash(&pool, "t-old", "p-old").await;
        let mut tx = crate::db::begin_write(&pool).await.unwrap();
        assert!(restore(&mut tx, "t-old").await.unwrap().is_empty());
        assert!(restore_blobs(&mut tx, "t-old").await.unwrap().is_empty());
    }

    #[test]
    fn ident_refuses_anything_but_a_plain_name() {
        assert_eq!(ident("photo_hash").unwrap(), "\"photo_hash\"");
        for bad in ["", "a b", "x\"; DROP TABLE photos; --", "a-b", "é"] {
            assert!(ident(bad).is_err(), "{bad:?} accepted");
        }
    }
}
