//! Reclaim orphan thumbnail blobs into a reversible quarantine (TODO.md F1).
//!
//! # Where the orphans come from
//!
//! A phone re-uploading a library the server already holds uploads the media
//! (the server dedups it by `content_hash` and keeps nothing), then uploads a
//! thumbnail (encrypted thumbnails never hash-match, so it IS stored), then
//! registers — and `register-encrypted` answers "already exists" with the
//! existing photo's ids. The new thumbnail is referenced by nothing. CT132 held
//! 18,067 of them (754 MB) on 2026-11-25.
//!
//! # Why thumbnails only
//!
//! "No `photos` row" is NOT "orphan" for media: web's full sync renders an
//! unregistered media blob as a photo (`syncDirectBlobs` in
//! `web/src/gallery/hooks/syncPass.ts`). Those "direct blobs" name their
//! thumbnail only inside their encrypted payload, which the server cannot
//! read — so their thumbnails look exactly like orphans. The only server-side
//! signal is time: a client uploads a thumbnail and its media back to back. A
//! thumbnail uploaded within [`DIRECT_MEDIA_WINDOW_DAYS`] of an unreferenced
//! media blob of the same user is kept and counted, never quarantined.
//!
//! # Never unlink here
//!
//! [`sweep`] moves the `blobs` row into `quarantined_blobs` (migration 040) and
//! the file under `<root>/.quarantine/<run_id>/`. [`restore`] reverses a run
//! exactly. [`purge`] is the only unlink, needs the run id and a matching count.
//! All three are dry runs unless `apply` is set.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::error::AppError;

/// Blob types this sweeper may quarantine. Media, manifests, motion videos and
/// renditions are never candidates (see the module docs).
pub const THUMB_TYPES: &[&str] = &["thumbnail", "video_thumbnail"];

/// Media types a web client renders even with no `photos` row.
pub const DIRECT_MEDIA_TYPES: &[&str] = &["photo", "gif", "video", "audio"];

/// An upload in flight is unreferenced until its registration lands, and the
/// Android client uploads + registers in one call (a retry re-uploads). A day
/// is orders of magnitude past any legitimate gap.
pub const GRACE_DAYS: f64 = 1.0;

/// ±1h around an unreferenced media blob of the same user.
pub const DIRECT_MEDIA_WINDOW_DAYS: f64 = 1.0 / 24.0;

pub const DEFAULT_APPLY_LIMIT: u32 = 1000;
pub const MAX_APPLY_LIMIT: u32 = 5000;

/// Lives inside the storage root so a rename never crosses a filesystem.
/// Autoscan skips dot-directories (`backup/autoscan.rs`), so nothing in here is
/// ever re-imported.
pub const QUARANTINE_DIR: &str = ".quarantine";

/// Every `(table, column)` that can hold a blob id. A blob any of these names
/// is live. `reference_columns_cover_the_schema` fails when a migration adds a
/// `*blob*` / `*ref*` column this list does not know — add it here, or to the
/// test's `NON_REFERENCE_COLUMNS` if it does not hold a blob id.
pub const REFERENCE_COLUMNS: &[(&str, &str)] = &[
    // Client-upload paths give a photo its blob's id (shared namespace).
    ("photos", "id"),
    ("photos", "encrypted_blob_id"),
    ("photos", "encrypted_thumb_blob_id"),
    ("photos", "motion_video_blob_id"),
    ("encrypted_gallery_items", "blob_id"),
    ("encrypted_gallery_items", "original_blob_id"),
    ("encrypted_gallery_items", "encrypted_blob_id"),
    ("encrypted_gallery_items", "encrypted_thumb_blob_id"),
    ("video_renditions", "blob_id"),
    // Encrypted trash rows use the blob id as photo_id.
    ("trash_items", "photo_id"),
    ("trash_items", "encrypted_blob_id"),
    ("trash_items", "thumbnail_blob_id"),
    ("trash_items", "encrypted_thumb_blob_id"),
    ("photo_metadata", "blob_id"),
    ("shared_album_photos", "photo_ref"),
    ("orphaned_rendition_blobs", "blob_id"),
];

/// Every `(table, column)` holding a storage-root-relative file path, besides
/// `blobs.storage_path` itself. Ids are not the only link to a file: encrypted
/// trash restore re-creates a thumbnail blob AT `trash_items.thumb_path`. A
/// thumbnail whose file one of these names — or a second `blobs` row names — is
/// kept. `path_columns_cover_the_schema` fails when a migration adds a `*path*`
/// column this list does not know.
pub const PATH_COLUMNS: &[(&str, &str)] = &[
    ("photos", "file_path"),
    ("photos", "thumb_path"),
    ("photos", "source_path"),
    ("photo_metadata", "storage_path"),
    ("trash_items", "file_path"),
    ("trash_items", "thumb_path"),
    ("trash_items", "original_file_path"),
    ("export_files", "file_path"),
    ("scan_skipped_paths", "rel_path"),
    ("video_renditions", "file_path"),
];

/// Every path some other row names (one row each, may repeat): the
/// [`PATH_COLUMNS`], plus any `storage_path` two `blobs` rows share.
fn shared_paths_sql() -> String {
    let mut parts: Vec<String> = PATH_COLUMNS
        .iter()
        .map(|(t, c)| format!("SELECT {c} AS p FROM {t} WHERE {c} IS NOT NULL AND {c} <> ''"))
        .collect();
    parts.push(
        "SELECT storage_path AS p FROM blobs GROUP BY storage_path HAVING COUNT(*) > 1".into(),
    );
    parts.join(" UNION ALL ")
}

/// True when the `blobs` row in scope (alias `blobs`) has a file some other row
/// names. Used inside the move transaction, so it sees the same state it acts on.
fn path_used_elsewhere_sql() -> String {
    let mut parts: Vec<String> = PATH_COLUMNS
        .iter()
        .map(|(t, c)| {
            format!(
                "EXISTS(SELECT 1 FROM {t} WHERE REPLACE({c}, '\\', '/') = \
                 REPLACE(blobs.storage_path, '\\', '/'))"
            )
        })
        .collect();
    parts.push(
        "EXISTS(SELECT 1 FROM blobs b2 WHERE b2.storage_path = blobs.storage_path \
         AND b2.id <> blobs.id)"
            .into(),
    );
    parts.join(" OR ")
}

/// Paths compare with forward slashes: some rows were written on Windows.
fn normalize_path(p: &str) -> String {
    p.replace('\\', "/")
}

/// `'a','b'` for a compile-time list. Only ever fed the constants above.
fn sql_list(items: &[&str]) -> String {
    items
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Every referenced id, one row each (may repeat). NULLs are filtered: a NULL
/// inside a `NOT IN` subquery makes the whole predicate NULL, which would
/// silently report zero orphans.
fn referenced_ids_sql() -> String {
    REFERENCE_COLUMNS
        .iter()
        .map(|(t, c)| format!("SELECT {c} AS id FROM {t} WHERE {c} IS NOT NULL"))
        .collect::<Vec<_>>()
        .join(" UNION ALL ")
}

/// True when anything references the blob id bound as `?1`.
fn referenced_by_any_sql() -> String {
    REFERENCE_COLUMNS
        .iter()
        .map(|(t, c)| format!("EXISTS(SELECT 1 FROM {t} WHERE {c} = ?1)"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct UnreferencedThumb {
    id: String,
    user_id: String,
    size_bytes: i64,
    storage_path: String,
    upload_jd: Option<f64>,
}

/// Why an unreferenced thumbnail is or is not quarantined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Candidate,
    /// Younger than [`GRACE_DAYS`], or its `upload_time` does not parse.
    Young,
    /// Possibly the thumbnail of a web "direct blob".
    NearDirectMedia,
    /// `storage_path` is empty, absolute, or escapes the root.
    BadPath,
    /// Another row names this file by path (see [`PATH_COLUMNS`]).
    SharedPath,
}

/// `storage_path` as a path that stays inside the root, or `None`.
fn safe_relative(storage_path: &str) -> Option<PathBuf> {
    if storage_path.is_empty() {
        return None;
    }
    let p = Path::new(storage_path);
    if p.components().all(|c| matches!(c, Component::Normal(_))) {
        Some(p.to_path_buf())
    } else {
        None
    }
}

/// Pure classification. `direct_media` is `(user_id, julianday(upload_time))`
/// of every unreferenced media blob; an unparseable media time keeps all of
/// that user's thumbnails (fail closed). `path_shared`: another row names this
/// file (see [`PATH_COLUMNS`]).
pub fn classify(
    user_id: &str,
    storage_path: &str,
    upload_jd: Option<f64>,
    now_jd: f64,
    direct_media: &[(String, Option<f64>)],
    path_shared: bool,
) -> Verdict {
    if safe_relative(storage_path).is_none() {
        return Verdict::BadPath;
    }
    if path_shared {
        return Verdict::SharedPath;
    }
    let Some(jd) = upload_jd else {
        return Verdict::Young;
    };
    if now_jd - jd < GRACE_DAYS {
        return Verdict::Young;
    }
    let near = direct_media.iter().any(|(u, mjd)| {
        u == user_id
            && match mjd {
                Some(m) => (m - jd).abs() <= DIRECT_MEDIA_WINDOW_DAYS,
                None => true,
            }
    });
    if near {
        Verdict::NearDirectMedia
    } else {
        Verdict::Candidate
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SweepOptions {
    pub apply: bool,
    /// Must equal the current `candidates` when `apply` is set.
    pub confirm_count: Option<u64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct SweepReport {
    pub dry_run: bool,
    /// Thumbnails nothing references, before any keep rule.
    pub unreferenced: u64,
    pub candidates: u64,
    pub candidate_bytes: i64,
    pub kept_young: u64,
    pub kept_near_direct_media: u64,
    pub kept_bad_path: u64,
    pub kept_shared_path: u64,
    /// Unreferenced media blobs (web shows them); never touched.
    pub direct_media_blobs: u64,
    pub quarantined: u64,
    pub quarantined_bytes: i64,
    /// Rows quarantined whose file was already gone.
    pub missing_file: u64,
    /// Candidates that gained a reference between the scan and the move.
    pub became_referenced: u64,
    pub failed: u64,
    /// Candidates left for the next apply call (limit).
    pub remaining: u64,
    pub run_id: Option<String>,
    pub quarantine_dir: Option<String>,
    /// Up to 20 candidate ids, oldest first, for spot checks.
    pub sample: Vec<String>,
}

async fn refuse_on_backup_mode(pool: &sqlx::SqlitePool) -> Result<(), AppError> {
    let mode: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_settings WHERE key = 'backup_mode'")
            .fetch_optional(pool)
            .await?;
    if mode.as_deref() == Some("backup") {
        return Err(AppError::Conflict(
            "Refusing: this is a backup server — blobs arrive before their photo rows, so \
             'unreferenced' is not 'orphan' here"
                .into(),
        ));
    }
    Ok(())
}

/// Find orphan thumbnails; with `apply`, quarantine up to `limit` of them.
pub async fn sweep(
    pool: &sqlx::SqlitePool,
    storage_root: &Path,
    opts: SweepOptions,
) -> Result<SweepReport, AppError> {
    refuse_on_backup_mode(pool).await?;

    let refs = referenced_ids_sql();
    let thumbs = sql_list(THUMB_TYPES);
    let media = sql_list(DIRECT_MEDIA_TYPES);

    let now_jd: f64 = sqlx::query_scalar("SELECT julianday('now')")
        .fetch_one(pool)
        .await?;

    let unreferenced: Vec<UnreferencedThumb> = sqlx::query_as(&format!(
        "SELECT id, user_id, size_bytes, storage_path, julianday(upload_time) AS upload_jd \
         FROM blobs WHERE blob_type IN ({thumbs}) \
         AND id NOT IN (SELECT id FROM ({refs})) \
         ORDER BY upload_time ASC, id ASC"
    ))
    .fetch_all(pool)
    .await?;

    let direct_media: Vec<(String, Option<f64>)> = sqlx::query_as(&format!(
        "SELECT user_id, julianday(upload_time) FROM blobs WHERE blob_type IN ({media}) \
         AND id NOT IN (SELECT id FROM ({refs}))"
    ))
    .fetch_all(pool)
    .await?;

    // An empty reference set makes EVERY thumbnail an orphan. A real library
    // with thumbnails and not one referenced blob is a broken query or a
    // half-restored database, not a sweep target.
    let referenced_blobs: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM blobs WHERE id IN (SELECT id FROM ({refs}))"
    ))
    .fetch_one(pool)
    .await?;
    if !unreferenced.is_empty() && referenced_blobs == 0 {
        tracing::error!(
            unreferenced = unreferenced.len(),
            "[ORPHAN-THUMBS] refusing: no blob is referenced by anything"
        );
        return Err(AppError::Conflict(format!(
            "Refusing: {} thumbnails look orphaned but no blob at all is referenced — the \
             reference set is empty, which means a broken library, not orphans",
            unreferenced.len()
        )));
    }

    let shared_paths: std::collections::HashSet<String> =
        sqlx::query_scalar::<_, String>(&shared_paths_sql())
            .fetch_all(pool)
            .await?
            .iter()
            .map(|p| normalize_path(p))
            .collect();

    let mut report = SweepReport {
        dry_run: !opts.apply,
        unreferenced: unreferenced.len() as u64,
        direct_media_blobs: direct_media.len() as u64,
        ..Default::default()
    };
    let mut candidates: Vec<UnreferencedThumb> = Vec::new();
    for row in unreferenced {
        match classify(
            &row.user_id,
            &row.storage_path,
            row.upload_jd,
            now_jd,
            &direct_media,
            shared_paths.contains(&normalize_path(&row.storage_path)),
        ) {
            Verdict::Candidate => {
                report.candidate_bytes += row.size_bytes;
                candidates.push(row);
            }
            Verdict::Young => report.kept_young += 1,
            Verdict::NearDirectMedia => report.kept_near_direct_media += 1,
            Verdict::BadPath => {
                tracing::warn!(blob_id = %row.id, storage_path = %row.storage_path,
                    "[ORPHAN-THUMBS] kept: storage_path is not a safe relative path");
                report.kept_bad_path += 1;
            }
            Verdict::SharedPath => {
                tracing::warn!(blob_id = %row.id, storage_path = %row.storage_path,
                    "[ORPHAN-THUMBS] kept: another row names this file by path");
                report.kept_shared_path += 1;
            }
        }
    }
    report.candidates = candidates.len() as u64;
    report.sample = candidates.iter().take(20).map(|r| r.id.clone()).collect();

    tracing::info!(
        apply = opts.apply,
        unreferenced = report.unreferenced,
        candidates = report.candidates,
        candidate_bytes = report.candidate_bytes,
        kept_young = report.kept_young,
        kept_near_direct_media = report.kept_near_direct_media,
        kept_bad_path = report.kept_bad_path,
        kept_shared_path = report.kept_shared_path,
        "[ORPHAN-THUMBS] scan complete"
    );

    if !opts.apply {
        report.remaining = report.candidates;
        return Ok(report);
    }

    match opts.confirm_count {
        None => {
            return Err(AppError::BadRequest(format!(
                "apply needs confirm_count = the candidate count ({}) from a dry run",
                report.candidates
            )))
        }
        Some(n) if n != report.candidates => {
            return Err(AppError::Conflict(format!(
                "confirm_count {n} does not match the current {} candidates — re-run the dry run",
                report.candidates
            )))
        }
        Some(_) => {}
    }

    let limit = opts
        .limit
        .unwrap_or(DEFAULT_APPLY_LIMIT)
        .clamp(1, MAX_APPLY_LIMIT) as usize;
    let run_id = uuid::Uuid::new_v4().to_string();
    let qdir = format!("{QUARANTINE_DIR}/{run_id}");
    tracing::info!(run_id = %run_id, quarantine_dir = %qdir, batch = limit.min(candidates.len()),
        "[ORPHAN-THUMBS] quarantining (restore: POST /api/admin/maintenance/orphan-thumbnails/restore)");

    let batch = &candidates[..limit.min(candidates.len())];
    for row in batch {
        match quarantine_one(pool, storage_root, &run_id, row).await {
            Ok(QuarantineOutcome::Moved) => {
                report.quarantined += 1;
                report.quarantined_bytes += row.size_bytes;
            }
            Ok(QuarantineOutcome::MissingFile) => {
                report.quarantined += 1;
                report.missing_file += 1;
            }
            Ok(QuarantineOutcome::BecameReferenced) => report.became_referenced += 1,
            Err(e) => {
                report.failed += 1;
                tracing::error!(blob_id = %row.id, "[ORPHAN-THUMBS] quarantine failed: {e}");
            }
        }
    }
    report.remaining = (candidates.len() - batch.len()) as u64;
    report.run_id = Some(run_id.clone());
    report.quarantine_dir = Some(qdir);

    tracing::info!(
        run_id = %run_id,
        quarantined = report.quarantined,
        quarantined_bytes = report.quarantined_bytes,
        missing_file = report.missing_file,
        became_referenced = report.became_referenced,
        failed = report.failed,
        remaining = report.remaining,
        "[ORPHAN-THUMBS] quarantine run complete"
    );
    Ok(report)
}

enum QuarantineOutcome {
    Moved,
    MissingFile,
    BecameReferenced,
}

const BLOB_COLUMNS: &str = "id, user_id, blob_type, size_bytes, client_hash, upload_time, \
                            storage_path, content_hash, blob_format";

async fn quarantine_one(
    pool: &sqlx::SqlitePool,
    root: &Path,
    run_id: &str,
    row: &UnreferencedThumb,
) -> Result<QuarantineOutcome, AppError> {
    let rel = safe_relative(&row.storage_path)
        .ok_or_else(|| AppError::Internal("unsafe storage_path reached quarantine".into()))?;
    let qrel = format!("{QUARANTINE_DIR}/{run_id}/{}", row.storage_path);
    let now = chrono::Utc::now().to_rfc3339();

    // Row first, in one transaction that re-checks the whole predicate: a
    // reference (by id or by path) that appeared since the scan makes this a no-op.
    let mut tx = pool.begin().await?;
    let copied = sqlx::query(&format!(
        "INSERT INTO quarantined_blobs ({BLOB_COLUMNS}, run_id, quarantine_path, quarantined_at, reason) \
         SELECT {BLOB_COLUMNS}, ?2, ?3, ?4, 'orphan_thumbnail' FROM blobs \
         WHERE id = ?1 AND blob_type IN ({}) AND NOT ({}) AND NOT ({})",
        sql_list(THUMB_TYPES),
        referenced_by_any_sql(),
        path_used_elsewhere_sql()
    ))
    .bind(&row.id)
    .bind(run_id)
    .bind(&qrel)
    .bind(&now)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if copied == 0 {
        tx.rollback().await?;
        return Ok(QuarantineOutcome::BecameReferenced);
    }
    sqlx::query("DELETE FROM blobs WHERE id = ?")
        .bind(&row.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let src = root.join(&rel);
    let dst = root.join(&qrel);
    match tokio::fs::metadata(&src).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(blob_id = %row.id, "[ORPHAN-THUMBS] file already gone; row quarantined");
            return Ok(QuarantineOutcome::MissingFile);
        }
        Err(e) => {
            unquarantine_row(pool, &row.id).await?;
            return Err(AppError::Internal(format!("stat {}: {e}", src.display())));
        }
    }
    let moved = async {
        if let Some(parent) = dst.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(&src, &dst).await
    }
    .await;
    if let Err(e) = moved {
        // The file never left, so put the row back where it was.
        unquarantine_row(pool, &row.id).await?;
        return Err(AppError::Internal(format!(
            "move {} -> {}: {e} (row restored)",
            src.display(),
            dst.display()
        )));
    }
    Ok(QuarantineOutcome::Moved)
}

/// Move a quarantined row back into `blobs`. Refuses to overwrite a live row,
/// and then keeps the quarantine record: it is the only thing naming the file.
async fn unquarantine_row(pool: &sqlx::SqlitePool, id: &str) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(&format!(
        "INSERT INTO blobs ({BLOB_COLUMNS}) SELECT {BLOB_COLUMNS} FROM quarantined_blobs \
         WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM blobs WHERE id = ?1)"
    ))
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        tx.rollback().await?;
        return Err(AppError::Conflict(format!(
            "blob {id} is live again (or not quarantined); quarantine record kept"
        )));
    }
    sqlx::query("DELETE FROM quarantined_blobs WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

fn parse_run_id(run_id: &str) -> Result<String, AppError> {
    if run_id.trim().is_empty() {
        return Err(AppError::BadRequest("run_id is required".into()));
    }
    uuid::Uuid::parse_str(run_id)
        .map(|u| u.to_string())
        .map_err(|_| AppError::BadRequest("run_id must be a run id returned by apply".into()))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct QuarantinedRow {
    id: String,
    size_bytes: i64,
    storage_path: String,
    quarantine_path: String,
}

async fn run_rows(pool: &sqlx::SqlitePool, run_id: &str) -> Result<Vec<QuarantinedRow>, AppError> {
    Ok(sqlx::query_as(
        "SELECT id, size_bytes, storage_path, quarantine_path FROM quarantined_blobs \
         WHERE run_id = ? ORDER BY id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?)
}

/// The quarantine path, but only if it is inside `.quarantine/<run_id>/`.
fn safe_quarantine_path(run_id: &str, quarantine_path: &str) -> Option<PathBuf> {
    let p = safe_relative(quarantine_path)?;
    let prefix = Path::new(QUARANTINE_DIR).join(run_id);
    (p.starts_with(&prefix) && p != prefix).then_some(p)
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct RestoreReport {
    pub dry_run: bool,
    pub run_id: String,
    pub rows: u64,
    pub bytes: i64,
    pub restored: u64,
    /// A file already sits at the original path; left for a human.
    pub conflicts: u64,
    pub failed: u64,
}

/// Put a quarantine run back exactly as it was.
pub async fn restore(
    pool: &sqlx::SqlitePool,
    root: &Path,
    run_id: &str,
    apply: bool,
) -> Result<RestoreReport, AppError> {
    let run_id = parse_run_id(run_id)?;
    let rows = run_rows(pool, &run_id).await?;
    let mut report = RestoreReport {
        dry_run: !apply,
        run_id: run_id.clone(),
        rows: rows.len() as u64,
        bytes: rows.iter().map(|r| r.size_bytes).sum(),
        ..Default::default()
    };
    if !apply {
        return Ok(report);
    }
    for row in &rows {
        match restore_one(pool, root, &run_id, row).await {
            Ok(true) => report.restored += 1,
            Ok(false) => report.conflicts += 1,
            Err(e) => {
                report.failed += 1;
                tracing::error!(blob_id = %row.id, run_id = %run_id, "[ORPHAN-THUMBS] restore failed: {e}");
            }
        }
    }
    tracing::info!(run_id = %run_id, restored = report.restored, conflicts = report.conflicts,
        failed = report.failed, "[ORPHAN-THUMBS] restore complete");
    Ok(report)
}

async fn restore_one(
    pool: &sqlx::SqlitePool,
    root: &Path,
    run_id: &str,
    row: &QuarantinedRow,
) -> Result<bool, AppError> {
    let orig = safe_relative(&row.storage_path)
        .ok_or_else(|| AppError::Internal("unsafe original storage_path".into()))?;
    let q = safe_quarantine_path(run_id, &row.quarantine_path)
        .ok_or_else(|| AppError::Internal("quarantine_path outside the run dir".into()))?;
    let (src, dst) = (root.join(&q), root.join(&orig));
    let io = |what: &str, p: &Path, e: std::io::Error| {
        AppError::Internal(format!("{what} {}: {e}", p.display()))
    };
    // A live row under this id would block the row restore — check before the
    // file moves, so a conflict leaves the quarantine exactly as it was.
    let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM blobs WHERE id = ?)")
        .bind(&row.id)
        .fetch_one(pool)
        .await?;
    if live {
        tracing::warn!(blob_id = %row.id, "[ORPHAN-THUMBS] restore conflict: a live row has this id");
        return Ok(false);
    }
    let in_quarantine = tokio::fs::try_exists(&src)
        .await
        .map_err(|e| io("stat", &src, e))?;
    if in_quarantine {
        if tokio::fs::try_exists(&dst)
            .await
            .map_err(|e| io("stat", &dst, e))?
        {
            tracing::warn!(blob_id = %row.id, "[ORPHAN-THUMBS] restore conflict: original path occupied");
            return Ok(false);
        }
        if let Some(parent) = dst.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| io("mkdir", parent, e))?;
        }
        tokio::fs::rename(&src, &dst)
            .await
            .map_err(|e| io("move back", &src, e))?;
    }
    // File back (or never moved): now the row. Idempotent on a re-run.
    unquarantine_row(pool, &row.id).await?;
    Ok(true)
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct PurgeReport {
    pub dry_run: bool,
    pub run_id: String,
    pub rows: u64,
    pub bytes: i64,
    pub purged: u64,
    /// A live `blobs` row has the same id — never unlinked.
    pub skipped_live: u64,
    pub failed: u64,
}

/// Permanently delete one quarantine run. The only unlink in this module.
pub async fn purge(
    pool: &sqlx::SqlitePool,
    root: &Path,
    run_id: &str,
    apply: bool,
    confirm_count: Option<u64>,
) -> Result<PurgeReport, AppError> {
    let run_id = parse_run_id(run_id)?;
    let rows = run_rows(pool, &run_id).await?;
    let mut report = PurgeReport {
        dry_run: !apply,
        run_id: run_id.clone(),
        rows: rows.len() as u64,
        bytes: rows.iter().map(|r| r.size_bytes).sum(),
        ..Default::default()
    };
    if !apply {
        return Ok(report);
    }
    if confirm_count != Some(report.rows) {
        return Err(AppError::Conflict(format!(
            "purge needs confirm_count = {} (the rows in run {run_id})",
            report.rows
        )));
    }
    tracing::warn!(run_id = %run_id, rows = report.rows, bytes = report.bytes,
        "[ORPHAN-THUMBS] purging quarantine run — not recoverable after this");
    for row in &rows {
        match purge_one(pool, root, &run_id, row).await {
            Ok(true) => report.purged += 1,
            Ok(false) => report.skipped_live += 1,
            Err(e) => {
                report.failed += 1;
                tracing::error!(blob_id = %row.id, run_id = %run_id, "[ORPHAN-THUMBS] purge failed: {e}");
            }
        }
    }
    tracing::warn!(run_id = %run_id, purged = report.purged, skipped_live = report.skipped_live,
        failed = report.failed, "[ORPHAN-THUMBS] purge complete");
    Ok(report)
}

async fn purge_one(
    pool: &sqlx::SqlitePool,
    root: &Path,
    run_id: &str,
    row: &QuarantinedRow,
) -> Result<bool, AppError> {
    let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM blobs WHERE id = ?)")
        .bind(&row.id)
        .fetch_one(pool)
        .await?;
    if live {
        tracing::warn!(blob_id = %row.id, "[ORPHAN-THUMBS] purge skipped: blob is live again");
        return Ok(false);
    }
    let q = safe_quarantine_path(run_id, &row.quarantine_path)
        .ok_or_else(|| AppError::Internal("quarantine_path outside the run dir".into()))?;
    match tokio::fs::remove_file(root.join(&q)).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(AppError::Internal(format!("unlink {}: {e}", q.display()))),
    }
    sqlx::query("DELETE FROM quarantined_blobs WHERE id = ? AND run_id = ?")
        .bind(&row.id)
        .bind(run_id)
        .execute(pool)
        .await?;
    Ok(true)
}

#[cfg(test)]
#[path = "orphan_thumbs_tests.rs"]
mod tests;
