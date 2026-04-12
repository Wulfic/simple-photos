//! Server-side parallel encryption migration — orchestration and entry points.
//!
//! On startup (and after each autoscan), if any photos have
//! `encrypted_blob_id IS NULL`, this module auto-migrates them by delegating
//! to [`super::server_migrate_encrypt`] for the actual encryption pipeline.
//!
//! This file handles: progress tracking, parallel scheduling, and public
//! entry points.  The per-photo encryption logic lives in
//! [`server_migrate_encrypt`].

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::crypto;

use super::server_migrate_encrypt::{encrypt_one_photo, repair_encrypted_thumbnail_orientation, repair_missing_thumbnails, PlainPhotoRow};

// ── Shared migration progress (lock-free) ───────────────────────────────────

struct MigrationProgress {
    total: AtomicU64,
    completed: AtomicU64,
    succeeded: AtomicU64,
    failed: AtomicU64,
    running: AtomicBool,
    current_file: tokio::sync::RwLock<String>,
    last_error: tokio::sync::RwLock<String>,
}

impl MigrationProgress {
    fn new(total: u64) -> Self {
        Self {
            total: AtomicU64::new(total),
            completed: AtomicU64::new(0),
            succeeded: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            running: AtomicBool::new(true),
            current_file: tokio::sync::RwLock::new(String::new()),
            last_error: tokio::sync::RwLock::new(String::new()),
        }
    }
}

/// Global handle to the current migration (if any).
static MIGRATION_PROGRESS: std::sync::OnceLock<tokio::sync::RwLock<Option<Arc<MigrationProgress>>>> =
    std::sync::OnceLock::new();

fn progress_store() -> &'static tokio::sync::RwLock<Option<Arc<MigrationProgress>>> {
    MIGRATION_PROGRESS.get_or_init(|| tokio::sync::RwLock::new(None))
}

// ── Parallel migration orchestrator ─────────────────────────────────────────

async fn run_migration(
    key: [u8; 32],
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
    progress: Arc<MigrationProgress>,
) {
    // Fetch all photos that haven't been encrypted yet.
    // Audio filtering happens at the intake points (autoscan + sync engine),
    // NOT here — if a file is already in the photos table it must be encrypted.
    let photos: Vec<PlainPhotoRow> = match sqlx::query_as::<_, PlainPhotoRow>(
        "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, created_at \
         FROM photos WHERE encrypted_blob_id IS NULL \
         ORDER BY created_at ASC",
    )
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Migration query failed: {}", e);
            *progress.last_error.write().await = format!("DB query failed: {}", e);
            progress.running.store(false, Ordering::Release);
            return;
        }
    };

    let total = photos.len() as u64;
    progress.total.store(total, Ordering::Release);
    tracing::info!("Server-side migration: {} photos to encrypt", total);

    if total == 0 {
        progress.running.store(false, Ordering::Release);
        return;
    }

    let parallelism = num_cpus::get().min(8).max(1);
    let semaphore = Arc::new(Semaphore::new(parallelism));
    tracing::info!("Migration parallelism: {} concurrent tasks", parallelism);

    let migration_start = std::time::Instant::now();
    let mut handles = Vec::with_capacity(photos.len());

    for photo in photos {
        let sem = semaphore.clone();
        let key_copy = key;
        let pool_clone = pool.clone();
        let root_clone = storage_root.clone();
        let progress_clone = progress.clone();
        let filename = photo.filename.clone();
        let handle = tokio::spawn(async move {
            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    progress_clone.failed.fetch_add(1, Ordering::Relaxed);
                    progress_clone.completed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };

            *progress_clone.current_file.write().await = filename.clone();
            let start = std::time::Instant::now();
            tracing::info!("[SERVER_MIG] start encrypting: {}", filename);

            match encrypt_one_photo(photo, &key_copy, &pool_clone, &root_clone).await {
                Ok(()) => {
                    let elapsed = start.elapsed();
                    tracing::info!(
                        "[SERVER_MIG] done encrypting: {} ({:.2}s)",
                        filename,
                        elapsed.as_secs_f64()
                    );
                    progress_clone.succeeded.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    let elapsed = start.elapsed();
                    tracing::error!(
                        "[SERVER_MIG] FAILED encrypting: {} ({:.2}s): {}",
                        filename,
                        elapsed.as_secs_f64(),
                        e
                    );
                    progress_clone.failed.fetch_add(1, Ordering::Relaxed);
                    *progress_clone.last_error.write().await = e;
                }
            }

            progress_clone.completed.fetch_add(1, Ordering::Relaxed);
        });

        handles.push(handle);
    }

    for handle in handles {
        if let Err(e) = handle.await {
            tracing::error!("Migration worker task panicked: {}", e);
        }
    }

    let wall_time = migration_start.elapsed();
    let succeeded = progress.succeeded.load(Ordering::Relaxed);
    let failed = progress.failed.load(Ordering::Relaxed);
    let completed = progress.completed.load(Ordering::Relaxed);
    let last_error = progress.last_error.read().await.clone();

    tracing::info!(
        "[SERVER_MIG] wall time: {:.2}s for {} photos ({} workers)",
        wall_time.as_secs_f64(),
        total,
        parallelism
    );
    tracing::info!(
        "Server-side migration complete: {}/{} succeeded, {} failed",
        succeeded,
        completed,
        failed
    );

    // Repair pass: fix photos with missing encrypted_thumb_blob_id
    repair_missing_thumbnails(key, &pool, &storage_root).await;

    // Repair pass: regenerate encrypted thumbnails with correct EXIF orientation
    repair_encrypted_thumbnail_orientation(key, &pool, &storage_root).await;

    if failed > 0 {
        tracing::warn!(
            "[SERVER_MIG] finished with {}/{} failures. Last: {}",
            failed,
            completed,
            last_error
        );
    }

    progress.running.store(false, Ordering::Release);
}

// ── Public entry points ─────────────────────────────────────────────────────

/// Count unencrypted photos eligible for migration.
async fn count_migratable(pool: &sqlx::SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NULL")
        .fetch_one(pool)
        .await
}

/// Start encryption migration for all unencrypted photos.
/// Called after autoscan and on startup.
///
/// Re-checks for newly arrived unencrypted photos after each run so that
/// files synced from a primary server during a migration batch are not left
/// behind.
pub async fn run_migration_from_stored_key(
    key: [u8; 32],
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
) {
    // Check if a migration is already running
    {
        let guard = progress_store().read().await;
        if let Some(ref p) = *guard {
            if p.running.load(Ordering::Acquire) {
                tracing::warn!("[SERVER_MIG] Migration already running, skipping");
                return;
            }
        }
    }

    loop {
        let count: i64 = match count_migratable(&pool).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("[SERVER_MIG] Failed to count photos: {}", e);
                return;
            }
        };

        if count == 0 {
            tracing::info!("[SERVER_MIG] No photos to migrate");
            return;
        }

        let progress = Arc::new(MigrationProgress::new(count as u64));
        {
            let mut guard = progress_store().write().await;
            *guard = Some(progress.clone());
        }

        tracing::info!(
            "[SERVER_MIG] Starting server-side migration for {} photos",
            count
        );

        run_migration(key, pool.clone(), storage_root.clone(), progress).await;

        // Re-check: photos may have arrived during the migration (e.g. backup sync).
        // If there are more, loop and process them immediately.
        let remaining: i64 = count_migratable(&pool).await.unwrap_or(0);

        if remaining == 0 {
            tracing::info!("[SERVER_MIG] All photos encrypted");
            crate::audit::log_background(
                &pool,
                crate::audit::AuditEvent::EncryptionMigrationComplete,
                Some(serde_json::json!({"migrated": count})),
            );
            return;
        }

        tracing::info!(
            "[SERVER_MIG] {} new unencrypted photos arrived during migration, re-running",
            remaining
        );
    }
}

/// Resume an interrupted encryption migration on server startup.
///
/// Checks if any unencrypted photos exist and a wrapped encryption key
/// is stored in the DB. If so, loads the key and resumes migration.
pub async fn resume_migration_on_startup(
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
    jwt_secret: String,
) {
    // Wait for the system to settle after startup
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;

    // Always attempt the one-time encrypted thumbnail orientation repair,
    // even if no new migration is needed — the repair fixes thumbnails
    // generated by a previous migration that did not apply EXIF orientation.
    if let Ok(Some(key)) = crypto::load_wrapped_key(&pool, &jwt_secret).await {
        repair_encrypted_thumbnail_orientation(key, &pool, &storage_root).await;
    }

    let unencrypted_count: i64 = count_migratable(&pool).await.unwrap_or(0);

    if unencrypted_count == 0 {
        tracing::debug!("[STARTUP_MIG] All photos encrypted, no migration needed");
        return;
    }

    let key = match crypto::load_wrapped_key(&pool, &jwt_secret).await {
        Ok(Some(k)) => k,
        Ok(None) => {
            tracing::warn!(
                "[STARTUP_MIG] {} unencrypted photos found but no stored key. \
                 A client must log in to provide the encryption key.",
                unencrypted_count
            );
            return;
        }
        Err(e) => {
            tracing::error!("[STARTUP_MIG] Failed to load stored encryption key: {}", e);
            return;
        }
    };

    tracing::info!(
        "[STARTUP_MIG] Resuming encryption migration: {} unencrypted photos",
        unencrypted_count
    );

    run_migration_from_stored_key(key, pool, storage_root).await;
}

/// Trigger migration after an autoscan finds new files.
/// Loads the stored key and encrypts any unencrypted photos.
pub async fn auto_migrate_after_scan(
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
    jwt_secret: String,
) {
    let unencrypted_count: i64 = count_migratable(&pool).await.unwrap_or(0);

    if unencrypted_count == 0 {
        return;
    }

    let key = match crypto::load_wrapped_key(&pool, &jwt_secret).await {
        Ok(Some(k)) => k,
        Ok(None) => {
            tracing::debug!(
                "[AUTOSCAN_MIG] {} unencrypted photos but no stored key, skipping",
                unencrypted_count
            );
            return;
        }
        Err(e) => {
            tracing::error!("[AUTOSCAN_MIG] Failed to load key: {}", e);
            return;
        }
    };

    tracing::info!(
        "[AUTOSCAN_MIG] Encrypting {} new photos after autoscan",
        unencrypted_count
    );

    run_migration_from_stored_key(key, pool, storage_root).await;
}
