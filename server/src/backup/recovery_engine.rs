//! Recovery bookkeeping shared by both ends of a disaster recovery.
//!
//! Recovery itself is **push-based**: the recovering primary asks the backup
//! (`recover_from_backup` in `recovery.rs`), the backup runs the ordinary sync
//! engine against it with `is_recovery = true` (`run_sync`), then reports back
//! through `/api/backup/recovery-callback`. Both sides record progress in
//! `backup_sync_log` through [`update_recovery_log`].
//!
//! This file used to also hold `run_recovery` — an older *pull*-based engine
//! (download each missing photo from `/backup/download/{id}`, insert it here).
//! Nothing had called it since the push design replaced it, and its INSERT had
//! been spliced into itself (`… AND id != ?1sh, crop_metadata) SELECT …`), so
//! it could not have run anyway. Removed in F10 (2026-11-25); recover it with
//! `git show 4a8d15b:server/src/backup/recovery_engine.rs` if ever needed —
//! but note it wrote each file BEFORE checking whether a live row owned that
//! path, then deleted it on a skip, and it could not restore `file_path = ''`
//! (Android) rows at all.

use chrono::Utc;

/// Update the sync log entry used for tracking recovery progress.
pub(crate) async fn update_recovery_log(
    pool: &sqlx::SqlitePool,
    log_id: &str,
    status: &str,
    photos_synced: i64,
    bytes_synced: i64,
    error: Option<&str>,
) {
    let now = Utc::now().to_rfc3339();
    if let Err(e) = sqlx::query(
        "UPDATE backup_sync_log SET completed_at = ?, status = ?, photos_synced = ?, \
         bytes_synced = ?, error = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(status)
    .bind(photos_synced)
    .bind(bytes_synced)
    .bind(error)
    .bind(log_id)
    .execute(pool)
    .await
    {
        tracing::error!("Failed to update recovery log {}: {}", log_id, e);
    }
}
