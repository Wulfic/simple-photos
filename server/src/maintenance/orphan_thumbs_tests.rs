//! Tests for `orphan_thumbs`. Every guard has a test that feeds it the bad input
//! and asserts nothing moved.

use super::*;
use std::str::FromStr;

/// Columns whose names match `*blob*` / `*ref*` but that hold no blob id.
const NON_REFERENCE_COLUMNS: &[(&str, &str)] = &[
    ("blobs", "blob_type"),
    ("blobs", "blob_format"),
    ("shared_album_photos", "ref_type"),
    // JSON of the `blobs` row; its id is `trash_blob_snapshots.blob_id`.
    ("trash_blob_snapshots", "blob_row"),
];

/// The format CT132's rows actually carry (9-digit fraction + offset).
const OLD: &str = "2026-08-06T10:54:42.701761664+00:00";
const USER: &str = "f5f0a851-e627-48a5-aa77-1f2e5d868187";

type BlobRow = (
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    i64,
);

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

fn scratch_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("orphan-thumbs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A blob row plus its file (contents = `bytes:<id>`), at the real path shape.
async fn add_blob(
    pool: &sqlx::SqlitePool,
    root: &Path,
    id: &str,
    blob_type: &str,
    upload_time: &str,
) -> String {
    let rel = crate::blobs::storage::relative_path(USER, id);
    let full = root.join(&rel);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(&full, format!("bytes:{id}")).unwrap();
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, \
         storage_path, content_hash, blob_format) VALUES (?, ?, ?, 10, 'ch', ?, ?, 'cth', 2)",
    )
    .bind(id)
    .bind(USER)
    .bind(blob_type)
    .bind(upload_time)
    .bind(&rel)
    .execute(pool)
    .await
    .unwrap();
    rel
}

/// A registered photo (media + thumb), so the reference set is non-empty.
async fn add_registered_photo(pool: &sqlx::SqlitePool, root: &Path) {
    add_blob(pool, root, "live-media", "photo", OLD).await;
    add_blob(pool, root, "live-thumb", "thumbnail", OLD).await;
    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, mime_type, created_at, \
         encrypted_blob_id, encrypted_thumb_blob_id) \
         VALUES ('p-live', ?, 'a.jpg', 'image/jpeg', ?, 'live-media', 'live-thumb')",
    )
    .bind(USER)
    .bind(OLD)
    .execute(pool)
    .await
    .unwrap();
}

async fn count(pool: &sqlx::SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

async fn blob_live(pool: &sqlx::SqlitePool, id: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM blobs WHERE id = ?)")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn blob_row(pool: &sqlx::SqlitePool, id: &str) -> BlobRow {
    sqlx::query_as(&format!("SELECT {BLOB_COLUMNS} FROM blobs WHERE id = ?"))
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn dry() -> SweepOptions {
    SweepOptions {
        apply: false,
        confirm_count: None,
        limit: None,
    }
}

fn apply(n: u64, limit: Option<u32>) -> SweepOptions {
    SweepOptions {
        apply: true,
        confirm_count: Some(n),
        limit,
    }
}

fn quarantined_file(root: &Path, run: &str, id: &str) -> PathBuf {
    root.join(QUARANTINE_DIR)
        .join(run)
        .join(crate::blobs::storage::relative_path(USER, id))
}

async fn columns_of(pool: &sqlx::SqlitePool, table: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
        .bind(table)
        .fetch_all(pool)
        .await
        .unwrap()
}

// ── Schema guards ───────────────────────────────────────────────────────

/// A migration that adds a blob-id column must teach the sweeper about it, or
/// the sweeper would quarantine blobs only that column references.
#[tokio::test]
async fn reference_columns_cover_the_schema() {
    let pool = test_pool().await;
    let found: Vec<(String, String)> = sqlx::query_as(
        "SELECT m.name, p.name FROM sqlite_master m, pragma_table_info(m.name) p \
         WHERE m.type = 'table' AND m.name NOT IN ('quarantined_blobs', '_sqlx_migrations') \
         AND (p.name LIKE '%blob%' OR p.name LIKE '%ref%')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let known = |t: &str, c: &str| {
        REFERENCE_COLUMNS.contains(&(t, c)) || NON_REFERENCE_COLUMNS.contains(&(t, c))
    };
    let unknown: Vec<_> = found.iter().filter(|(t, c)| !known(t, c)).collect();
    assert!(
        unknown.is_empty(),
        "blob/ref columns the sweeper does not know: {unknown:?}"
    );
    for (t, c) in REFERENCE_COLUMNS {
        assert!(
            columns_of(&pool, t).await.iter().any(|x| x == c),
            "REFERENCE_COLUMNS names {t}.{c}, which does not exist"
        );
    }
}

/// Restore copies `BLOB_COLUMNS` back. A `blobs` column missing from it (or
/// from the quarantine table) would be silently reset by a round trip.
#[tokio::test]
async fn quarantine_table_carries_every_blobs_column() {
    let pool = test_pool().await;
    let blobs = columns_of(&pool, "blobs").await;
    let quarantine = columns_of(&pool, "quarantined_blobs").await;
    let listed: Vec<&str> = BLOB_COLUMNS.split(',').map(str::trim).collect();
    for c in &blobs {
        assert!(quarantine.contains(c), "quarantined_blobs lacks blobs.{c}");
        assert!(listed.contains(&c.as_str()), "BLOB_COLUMNS lacks blobs.{c}");
    }
}

// ── What is and is not a candidate ──────────────────────────────────────

/// Every reference column, one at a time, keeps an otherwise-orphan thumbnail.
/// Rows are synthesised from each table's NOT NULL columns, so a column added
/// to REFERENCE_COLUMNS is covered without editing this test.
#[tokio::test]
async fn every_reference_column_keeps_its_blob() {
    for (table, column) in REFERENCE_COLUMNS {
        let pool = test_pool().await;
        let root = scratch_root();
        add_registered_photo(&pool, &root).await;
        add_blob(&pool, &root, "t-ref", "thumbnail", OLD).await;
        synthesise_row(&pool, table, column, "t-ref").await;

        let r = sweep(&pool, &root, dry()).await.unwrap();
        assert_eq!(
            (r.unreferenced, r.candidates),
            (0, 0),
            "{table}.{column} must keep its blob"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}

#[tokio::test]
async fn dry_run_reports_and_touches_nothing() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    let rel = add_blob(&pool, &root, "orphan-1", "thumbnail", OLD).await;
    // A NULL in a reference column must not blank the NOT IN (that would
    // silently report zero orphans).
    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, mime_type, created_at) \
         VALUES ('p-null', ?, 'b.jpg', 'image/jpeg', ?)",
    )
    .bind(USER)
    .bind(OLD)
    .execute(&pool)
    .await
    .unwrap();

    let r = sweep(&pool, &root, dry()).await.unwrap();

    assert!(r.dry_run);
    assert_eq!(
        (r.unreferenced, r.candidates, r.candidate_bytes),
        (1, 1, 10)
    );
    assert_eq!(r.sample, vec!["orphan-1".to_string()]);
    assert_eq!(r.remaining, 1);
    assert!(root.join(&rel).exists(), "dry run must not move the file");
    assert!(blob_live(&pool, "orphan-1").await);
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM quarantined_blobs").await,
        0
    );
    assert!(!root.join(QUARANTINE_DIR).exists());
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn young_and_unparseable_thumbnails_are_kept() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    let now = chrono::Utc::now().to_rfc3339();
    add_blob(&pool, &root, "young", "thumbnail", &now).await;
    add_blob(&pool, &root, "garbled", "video_thumbnail", "not a date").await;

    let r = sweep(&pool, &root, dry()).await.unwrap();

    assert_eq!((r.unreferenced, r.candidates, r.kept_young), (2, 0, 2));
    std::fs::remove_dir_all(&root).ok();
}

/// Web renders an unregistered media blob as a photo; its thumbnail is named
/// only inside the encrypted payload. Media is never a candidate, and a
/// thumbnail uploaded within the hour of it is kept.
#[tokio::test]
async fn direct_media_and_its_neighbouring_thumbnails_are_kept() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    add_blob(&pool, &root, "direct", "photo", "2026-08-06T10:00:00Z").await;
    add_blob(&pool, &root, "near", "thumbnail", "2026-08-06T10:59:00Z").await;
    add_blob(&pool, &root, "far", "thumbnail", "2026-08-06T11:30:00Z").await;

    let r = sweep(&pool, &root, dry()).await.unwrap();

    assert_eq!(r.direct_media_blobs, 1);
    assert_eq!(r.kept_near_direct_media, 1);
    assert_eq!(
        r.sample,
        vec!["far".to_string()],
        "only the far thumbnail goes"
    );
    assert!(blob_live(&pool, "direct").await);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn classify_edges() {
    let now = 2_461_000.0;
    let old = now - 30.0;
    let other_user = vec![("someone-else".to_string(), Some(old))];
    assert_eq!(
        classify(USER, "blobs/a.bin", Some(old), now, &other_user, false),
        Verdict::Candidate,
        "another user's direct media protects nothing of ours"
    );
    let garbled_media = vec![(USER.to_string(), None)];
    assert_eq!(
        classify(USER, "blobs/a.bin", Some(old), now, &garbled_media, false),
        Verdict::NearDirectMedia,
        "an unparseable media time fails closed"
    );
    assert_eq!(
        classify(USER, "blobs/a.bin", Some(now - 0.99), now, &[], false),
        Verdict::Young
    );
    for bad in ["", "../x.bin", "/etc/passwd", "blobs/../../x"] {
        assert_eq!(
            classify(USER, bad, Some(old), now, &[], true),
            Verdict::BadPath,
            "{bad:?}"
        );
    }
    assert_eq!(
        classify(USER, "blobs/a.bin", Some(old), now, &[], true),
        Verdict::SharedPath,
        "an old, id-orphan thumbnail whose file another row names is kept"
    );
}

/// A migration that adds a file-path column must teach the sweeper about it,
/// or a thumbnail linked to a row only by path would be quarantined.
#[tokio::test]
async fn path_columns_cover_the_schema() {
    let pool = test_pool().await;
    let found: Vec<(String, String)> = sqlx::query_as(
        "SELECT m.name, p.name FROM sqlite_master m, pragma_table_info(m.name) p \
         WHERE m.type = 'table' AND m.name NOT IN ('quarantined_blobs', '_sqlx_migrations') \
         AND p.name LIKE '%path%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let unknown: Vec<_> = found
        .iter()
        .filter(|(t, c)| {
            !PATH_COLUMNS.contains(&(t.as_str(), c.as_str()))
                && (t.as_str(), c.as_str()) != ("blobs", "storage_path")
        })
        .collect();
    assert!(
        unknown.is_empty(),
        "path columns the sweeper does not know: {unknown:?}"
    );
    for (t, c) in PATH_COLUMNS {
        assert!(
            columns_of(&pool, t).await.iter().any(|x| x == c),
            "PATH_COLUMNS names {t}.{c}, which does not exist"
        );
    }
}

/// Insert a row into `table` with `column = value`, filling every other
/// required column with a placeholder.
async fn synthesise_row(pool: &sqlx::SqlitePool, table: &str, column: &str, value: &str) {
    let cols: Vec<(String, String, bool, Option<String>, i64)> =
        sqlx::query_as("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?)")
            .bind(table)
            .fetch_all(pool)
            .await
            .unwrap();
    let mut names = vec![column.to_string()];
    let mut values = vec![format!("'{value}'")];
    for (name, ty, notnull, dflt, pk) in cols {
        if name == column || !(notnull || pk > 0) || dflt.is_some() {
            continue;
        }
        let ty = ty.to_uppercase();
        names.push(name.clone());
        values.push(if ty.contains("INT") || ty.contains("REAL") {
            "1".into()
        } else {
            format!("'{name}-x'")
        });
    }
    sqlx::query(&format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        names.join(","),
        values.join(",")
    ))
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("synthesising a {table} row: {e}"));
}

/// Every path column, one at a time, keeps an id-orphan thumbnail whose file it
/// names — in the scan, and in the move transaction (apply moves nothing).
#[tokio::test]
async fn every_path_column_keeps_its_file() {
    for (table, column) in PATH_COLUMNS {
        let pool = test_pool().await;
        let root = scratch_root();
        add_registered_photo(&pool, &root).await;
        let rel = add_blob(&pool, &root, "t-path", "thumbnail", OLD).await;
        // Stored with Windows separators: the guard must still match.
        synthesise_row(&pool, table, column, &rel.replace('/', "\\")).await;

        let r = sweep(&pool, &root, dry()).await.unwrap();
        assert_eq!(
            (r.unreferenced, r.kept_shared_path, r.candidates),
            (1, 1, 0),
            "{table}.{column} must keep the file it names"
        );
        assert!(root.join(&rel).exists());
        std::fs::remove_dir_all(&root).ok();
    }
}

/// Two `blobs` rows on one file: quarantining one would pull the other's file.
#[tokio::test]
async fn a_file_shared_by_two_blob_rows_is_kept() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    let rel = add_blob(&pool, &root, "orphan-1", "thumbnail", OLD).await;
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
         VALUES ('twin', ?, 'thumbnail', 1, ?, ?)",
    )
    .bind(USER)
    .bind(OLD)
    .bind(&rel)
    .execute(&pool)
    .await
    .unwrap();

    let r = sweep(&pool, &root, dry()).await.unwrap();

    assert_eq!(
        (r.unreferenced, r.kept_shared_path, r.candidates),
        (2, 2, 0)
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A path reference that lands between the scan and the move wins, like an id.
#[tokio::test]
async fn a_path_reference_that_appears_after_the_scan_wins() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    let rel = add_blob(&pool, &root, "late", "thumbnail", OLD).await;
    let row = UnreferencedThumb {
        id: "late".into(),
        user_id: USER.into(),
        size_bytes: 10,
        storage_path: rel.clone(),
        upload_jd: None,
    };
    sqlx::query("UPDATE photos SET thumb_path = ? WHERE id = 'p-live'")
        .bind(&rel)
        .execute(&pool)
        .await
        .unwrap();

    let run = uuid::Uuid::new_v4().to_string();
    let out = quarantine_one(&pool, &root, &run, &row).await.unwrap();

    assert!(matches!(out, QuarantineOutcome::BecameReferenced));
    assert!(blob_live(&pool, "late").await);
    assert!(root.join(&rel).exists());
    std::fs::remove_dir_all(&root).ok();
}

/// Encrypted trash restore re-inserts media + thumbnail blobs with one `now`
/// and no `photos` row: web shows the pair as a direct blob. Its thumbnail
/// must survive once it is past the grace period.
#[tokio::test]
async fn a_trash_restored_pair_keeps_its_thumbnail() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    add_blob(&pool, &root, "restored-media", "photo", OLD).await;
    add_blob(&pool, &root, "restored-thumb", "thumbnail", OLD).await;

    let r = sweep(&pool, &root, dry()).await.unwrap();

    assert_eq!(
        (r.unreferenced, r.kept_near_direct_media, r.candidates),
        (1, 1, 0)
    );
    std::fs::remove_dir_all(&root).ok();
}

// ── Refusals ────────────────────────────────────────────────────────────

/// No referenced blob at all ⇒ the reference set is broken, and every thumbnail
/// would look orphaned. Refuse, and move nothing even on apply.
#[tokio::test]
async fn empty_reference_set_refuses() {
    let pool = test_pool().await;
    let root = scratch_root();
    let rel = add_blob(&pool, &root, "lonely", "thumbnail", OLD).await;

    for opts in [dry(), apply(1, None)] {
        let err = sweep(&pool, &root, opts).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {err:?}");
    }
    assert!(root.join(&rel).exists());
    assert!(blob_live(&pool, "lonely").await);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn apply_needs_the_exact_candidate_count() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    let rel = add_blob(&pool, &root, "orphan-1", "thumbnail", OLD).await;

    let missing = SweepOptions {
        apply: true,
        confirm_count: None,
        limit: None,
    };
    assert!(matches!(
        sweep(&pool, &root, missing).await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        sweep(&pool, &root, apply(2, None)).await,
        Err(AppError::Conflict(_))
    ));

    assert!(root.join(&rel).exists());
    assert!(blob_live(&pool, "orphan-1").await);
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM quarantined_blobs").await,
        0
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn backup_mode_refuses() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    add_blob(&pool, &root, "orphan-1", "thumbnail", OLD).await;
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('backup_mode', 'backup') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(matches!(
        sweep(&pool, &root, apply(1, None)).await,
        Err(AppError::Conflict(_))
    ));
    assert!(blob_live(&pool, "orphan-1").await);
    std::fs::remove_dir_all(&root).ok();
}

// ── Quarantine, restore, purge ──────────────────────────────────────────

#[tokio::test]
async fn quarantine_then_restore_round_trips_exactly() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    let rel = add_blob(&pool, &root, "orphan-1", "thumbnail", OLD).await;
    let before = blob_row(&pool, "orphan-1").await;

    let r = sweep(&pool, &root, apply(1, None)).await.unwrap();
    let run_id = r.run_id.clone().unwrap();
    assert_eq!(
        (r.quarantined, r.quarantined_bytes, r.failed, r.remaining),
        (1, 10, 0, 0)
    );
    assert!(!blob_live(&pool, "orphan-1").await);
    assert!(!root.join(&rel).exists());
    let qfile = quarantined_file(&root, &run_id, "orphan-1");
    assert_eq!(std::fs::read_to_string(&qfile).unwrap(), "bytes:orphan-1");
    assert!(
        blob_live(&pool, "live-thumb").await,
        "a referenced thumbnail is untouched"
    );

    let dry_restore = restore(&pool, &root, &run_id, false).await.unwrap();
    assert_eq!((dry_restore.rows, dry_restore.restored), (1, 0));
    assert!(qfile.exists(), "a dry restore moves nothing");

    let back = restore(&pool, &root, &run_id, true).await.unwrap();
    assert_eq!((back.restored, back.conflicts, back.failed), (1, 0, 0));
    assert_eq!(
        before,
        blob_row(&pool, "orphan-1").await,
        "every blobs column survives the round trip"
    );
    assert_eq!(
        std::fs::read_to_string(root.join(&rel)).unwrap(),
        "bytes:orphan-1"
    );
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM quarantined_blobs").await,
        0
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn apply_honours_limit_and_reports_remaining() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    for i in 0..3 {
        add_blob(&pool, &root, &format!("orphan-{i}"), "thumbnail", OLD).await;
    }

    let r = sweep(&pool, &root, apply(3, Some(2))).await.unwrap();
    assert_eq!((r.quarantined, r.remaining), (2, 1));
    let next = sweep(&pool, &root, dry()).await.unwrap();
    assert_eq!(next.candidates, 1);
    std::fs::remove_dir_all(&root).ok();
}

/// A reference that lands between the scan and the move wins.
#[tokio::test]
async fn a_reference_that_appears_after_the_scan_wins() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    let rel = add_blob(&pool, &root, "late", "thumbnail", OLD).await;
    let row = UnreferencedThumb {
        id: "late".into(),
        user_id: USER.into(),
        size_bytes: 10,
        storage_path: rel.clone(),
        upload_jd: None,
    };
    sqlx::query("UPDATE photos SET encrypted_thumb_blob_id = 'late' WHERE id = 'p-live'")
        .execute(&pool)
        .await
        .unwrap();

    let run = uuid::Uuid::new_v4().to_string();
    let out = quarantine_one(&pool, &root, &run, &row).await.unwrap();

    assert!(matches!(out, QuarantineOutcome::BecameReferenced));
    assert!(blob_live(&pool, "late").await);
    assert!(root.join(&rel).exists());
    std::fs::remove_dir_all(&root).ok();
}

/// The file cannot be moved ⇒ the row goes back; nothing is half-quarantined.
#[tokio::test]
async fn a_failed_move_puts_the_row_back() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    let rel = add_blob(&pool, &root, "orphan-1", "thumbnail", OLD).await;
    let before = blob_row(&pool, "orphan-1").await;
    // A plain file where the quarantine dir must go makes create_dir_all fail.
    std::fs::write(root.join(QUARANTINE_DIR), b"in the way").unwrap();

    let r = sweep(&pool, &root, apply(1, None)).await.unwrap();

    assert_eq!((r.quarantined, r.failed), (0, 1));
    assert_eq!(
        before,
        blob_row(&pool, "orphan-1").await,
        "row must be restored intact"
    );
    assert!(root.join(&rel).exists());
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM quarantined_blobs").await,
        0
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn purge_refuses_without_run_id_or_count() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    add_blob(&pool, &root, "orphan-1", "thumbnail", OLD).await;
    let run_id = sweep(&pool, &root, apply(1, None))
        .await
        .unwrap()
        .run_id
        .unwrap();
    let qfile = quarantined_file(&root, &run_id, "orphan-1");

    for bad in ["", "   ", "../..", "*"] {
        assert!(
            matches!(
                purge(&pool, &root, bad, true, Some(1)).await,
                Err(AppError::BadRequest(_))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        purge(&pool, &root, &run_id, true, None).await,
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(
        purge(&pool, &root, &run_id, true, Some(7)).await,
        Err(AppError::Conflict(_))
    ));
    let dry_purge = purge(&pool, &root, &run_id, false, None).await.unwrap();
    assert_eq!((dry_purge.rows, dry_purge.purged), (1, 0));

    assert!(qfile.exists(), "no refused or dry purge may unlink");
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM quarantined_blobs").await,
        1
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn purge_unlinks_only_its_own_run_and_never_a_live_id() {
    let pool = test_pool().await;
    let root = scratch_root();
    add_registered_photo(&pool, &root).await;
    for i in 0..3 {
        add_blob(&pool, &root, &format!("orphan-{i}"), "thumbnail", OLD).await;
    }
    let run_a = sweep(&pool, &root, apply(3, Some(2)))
        .await
        .unwrap()
        .run_id
        .unwrap();
    let run_b = sweep(&pool, &root, apply(1, None))
        .await
        .unwrap()
        .run_id
        .unwrap();
    // Something re-created a live row under one of run A's ids.
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
         VALUES ('orphan-0', ?, 'thumbnail', 1, ?, 'blobs/elsewhere.bin')",
    )
    .bind(USER)
    .bind(OLD)
    .execute(&pool)
    .await
    .unwrap();

    let p = purge(&pool, &root, &run_a, true, Some(2)).await.unwrap();

    assert_eq!((p.purged, p.skipped_live, p.failed), (1, 1, 0));
    assert!(
        quarantined_file(&root, &run_a, "orphan-0").exists(),
        "a live id is never unlinked"
    );
    assert!(!quarantined_file(&root, &run_a, "orphan-1").exists());
    assert!(
        quarantined_file(&root, &run_b, "orphan-2").exists(),
        "another run is untouched"
    );
    // Restoring a run whose id is live again is a conflict, and the
    // quarantine record + file both stay (they are the only copy).
    let back = restore(&pool, &root, &run_a, true).await.unwrap();
    assert_eq!((back.restored, back.conflicts, back.failed), (0, 1, 0));
    assert!(quarantined_file(&root, &run_a, "orphan-0").exists());
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM quarantined_blobs").await,
        2
    );
    std::fs::remove_dir_all(&root).ok();
}
