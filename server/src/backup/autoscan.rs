//! Automatic filesystem scanner that registers new media files into the database.
//!
//! Runs as a background task on a configurable interval and can also be
//! triggered on-demand via `POST /api/admin/photos/auto-scan`.  Files are
//! assigned to the first admin user; duplicates are handled gracefully with
//! `INSERT OR IGNORE` to avoid race conditions with concurrent scans.
//!
//! After new files are registered, the scanner triggers the background
//! processing pipeline in [`crate::photos::convert`] which runs three
//! sequential phases: thumbnails → conversion → post-conversion thumbnails.
//!
//! In encrypted mode, newly registered files also trigger the encryption
//! migration if the encryption key is available.

use std::path::Path;

use axum::extract::State;
use axum::Json;
use futures_util::TryStreamExt;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::photos::metadata::extract_media_metadata;
use crate::photos::scan::{ffmpeg_available_pub, generate_thumbnail_file};
use crate::photos::utils::compute_photo_hash_streaming;
use crate::state::AppState;

// compute_photo_hash_streaming is now in photos::utils — imported above.

/// Background task: automatically scan the storage directory for new files
/// every 24 hours (or when triggered by an API call).
pub async fn background_auto_scan_task(
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
    interval_secs: u64,
    convert_notify: std::sync::Arc<tokio::sync::Notify>,
    encryption_key_store: std::sync::Arc<tokio::sync::RwLock<Option<[u8; 32]>>>,
    jwt_secret: String,
) {
    if interval_secs == 0 {
        tracing::info!("Background auto-scan disabled (interval = 0)");
        return;
    }

    // Run an initial scan shortly after startup
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    tracing::info!("[DIAG:AUTOSCAN] Running startup auto-scan...");
    let count = run_auto_scan(&pool, &storage_root).await;
    tracing::info!("[DIAG:AUTOSCAN] Startup auto-scan complete: registered {} new files", count);
    if count > 0 {
        auto_start_migration_if_needed(
            &pool, &storage_root, &convert_notify, &encryption_key_store, &jwt_secret,
        ).await;
    }
    update_last_scan_time(&pool).await;

    // Then scan on a configurable interval
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    tracing::info!("Auto-scan interval: every {} seconds", interval_secs);

    loop {
        interval.tick().await;

        let count = run_auto_scan(&pool, &storage_root).await;
        tracing::info!("[DIAG:AUTOSCAN] Interval auto-scan complete: registered {} new files", count);
        if count > 0 {
            auto_start_migration_if_needed(
                &pool, &storage_root, &convert_notify, &encryption_key_store, &jwt_secret,
            ).await;
        }
        update_last_scan_time(&pool).await;
    }
}

async fn update_last_scan_time(pool: &sqlx::SqlitePool) {
    let now = crate::photos::utils::utc_now_iso();
    let _ = sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('last_auto_scan', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&now)
    .execute(pool)
    .await;
}

/// POST /api/admin/photos/auto-scan
/// Trigger an immediate auto-scan (called when web UI or app opens).
/// Runs synchronously so the client can await completion before loading photos.
/// Admin only — the route is under `/api/admin/`.
pub async fn trigger_auto_scan(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    crate::setup::admin::require_admin(&state, &auth).await?;
    let pool = state.pool.clone();
    let storage_root = (**state.storage_root.load()).clone();

    let count = run_auto_scan(&pool, &storage_root).await;
    tracing::info!("[DIAG:AUTOSCAN] On-demand scan complete: registered {} new files", count);
    if count > 0 {
        auto_start_migration_if_needed(
            &pool,
            &storage_root,
            &state.convert_notify,
            &state.encryption_key,
            &state.config.auth.jwt_secret,
        ).await;
    }

    // Update last scan time
    update_last_scan_time(&pool).await;

    Ok(Json(serde_json::json!({
        "message": "Scan complete",
        "new_count": count,
    })))
}

/// If the server is in "encrypted" mode with an idle migration and there are
/// unencrypted plain photos, automatically start the encryption migration.
/// This resolves a race condition on fresh setup where the mode is set before
/// the initial scan registers any files.
///
/// When a stored encryption key is available, this also spawns the actual
/// migration task — making the entire process fully autonomous.
async fn auto_start_migration_if_needed(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
    convert_notify: &std::sync::Arc<tokio::sync::Notify>,
    encryption_key_store: &std::sync::Arc<tokio::sync::RwLock<Option<[u8; 32]>>>,
    jwt_secret: &str,
) {
    // Only relevant when mode is already "encrypted"
    let mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "plain".to_string());

    if mode != "encrypted" {
        tracing::info!("[DIAG:AUTOSCAN] auto_start_migration: mode='{}', skipping (not encrypted)", mode);
        return;
    }

    // Only act if no migration is already in progress
    let status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "idle".to_string());

    if status != "idle" {
        tracing::info!("[DIAG:AUTOSCAN] auto_start_migration: status='{}', skipping (not idle)", status);
        return;
    }

    // Count plain photos that need encryption.
    // Note: uses a global count (no user_id filter) because `run_migration` will
    // re-query per-user. This is a quick check to decide whether to start at all.
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    if count == 0 {
        tracing::info!("[DIAG:AUTOSCAN] auto_start_migration: 0 plain photos, nothing to do");
        return;
    }

    // Start the migration (set DB status)
    let now = crate::photos::utils::utc_now_iso();
    if let Err(e) = sqlx::query(
        "UPDATE encryption_migration SET status = 'encrypting', total = ?, completed = 0, \
         started_at = ?, error = NULL WHERE id = 'singleton'",
    )
    .bind(count)
    .bind(&now)
    .execute(pool)
    .await
    {
        tracing::error!("[DIAG:AUTOSCAN] Failed to start migration: {}", e);
        return;
    }

    tracing::info!(
        "[DIAG:AUTOSCAN] Auto-triggered encryption migration for {} unencrypted photos after scan",
        count
    );

    // Try to load the encryption key and spawn the actual migration task
    let key = {
        // First check in-memory store
        let guard = encryption_key_store.read().await;
        *guard
    };
    let key = match key {
        Some(k) => k,
        None => {
            // Try loading from DB
            match crate::crypto::load_wrapped_key(pool, jwt_secret).await {
                Ok(Some(k)) => {
                    let mut guard = encryption_key_store.write().await;
                    *guard = Some(k);
                    k
                }
                Ok(None) => {
                    tracing::warn!(
                        "[DIAG:AUTOSCAN] No encryption key available (in-memory or DB). \
                         Migration DB status set to 'encrypting' but actual task not started. \
                         A client must provide the key."
                    );
                    return;
                }
                Err(e) => {
                    tracing::error!("[DIAG:AUTOSCAN] Failed to load stored key: {}", e);
                    return;
                }
            }
        }
    };

    // Find the admin user
    let admin_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let user_id = match admin_id {
        Some(id) => id,
        None => {
            tracing::warn!("[DIAG:AUTOSCAN] No admin user found, cannot start migration task");
            return;
        }
    };

    // Spawn the migration in the background
    let pool_clone = pool.clone();
    let storage_root_clone = storage_root.to_path_buf();
    let convert_notify_clone = convert_notify.clone();
    let encryption_key_clone = encryption_key_store.clone();
    let jwt_secret_clone = jwt_secret.to_string();
    tokio::spawn(async move {
        crate::photos::server_migrate::run_migration_from_stored_key(
            key,
            user_id,
            pool_clone,
            storage_root_clone,
            convert_notify_clone,
            encryption_key_clone,
            jwt_secret_clone,
        )
        .await;
    });
}

/// Scan storage directory and register any unregistered media files for ALL users.
async fn run_auto_scan(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
) -> i64 {
    // Get the first admin user to assign new photos to
    let admin_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let admin_id = match admin_id {
        Some(id) => {
            tracing::info!("[DIAG:AUTOSCAN] run_auto_scan: admin_id={}", id);
            id
        }
        None => {
            tracing::info!("[DIAG:AUTOSCAN] run_auto_scan: no admin user yet, skipping");
            return 0;
        }
    };

    // Check whether audio files should be included in scan
    let audio_backup_enabled: bool = sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(false);

    // Check FFmpeg availability for thumbnail/preview generation
    let has_ffmpeg = ffmpeg_available_pub().await;

    // Build set of already-registered paths using a streaming cursor so we
    // never hold the full Vec<String> + HashSet simultaneously in memory.
    let mut existing_set = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM photos",
        )
        .fetch(pool);

        while let Some(path) = rows.try_next().await.unwrap_or(None) {
            existing_set.insert(path);
        }
    }
    tracing::info!("[DIAG:AUTOSCAN] run_auto_scan: {} existing photos in DB, scanning {:?}", existing_set.len(), storage_root);

    let mut new_count = 0i64;
    let mut queue = vec![storage_root.to_path_buf()];

    while let Some(dir) = queue.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    queue.push(entry.path());
                } else if ft.is_file() && is_media_file(&name) {
                    let abs_path = entry.path();
                    // Normalize to forward slashes so DB paths are consistent across OS
                    let rel_path = abs_path
                        .strip_prefix(storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    if existing_set.contains(&rel_path) {
                        continue;
                    }

                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            crate::photos::utils::normalize_iso_timestamp(&dt.to_rfc3339())
                        })
                    });

                    let mime = mime_from_extension(&name).to_string();
                    let media_type = if mime.starts_with("video/") {
                        "video"
                    } else if mime.starts_with("audio/") {
                        "audio"
                    } else if mime == "image/gif" {
                        "gif"
                    } else {
                        "photo"
                    };

                    // Skip audio files when audio backup is disabled
                    if media_type == "audio" && !audio_backup_enabled {
                        continue;
                    }

                    let photo_id = Uuid::new_v4().to_string();
                    let now = crate::photos::utils::utc_now_iso();
                    let thumb_rel = format!(".thumbnails/{}.thumb.jpg", photo_id);

                    // Extract dimensions, camera model, GPS, and date from file
                    // (matches scan_and_register behavior so photos have full metadata)
                    let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
                        extract_media_metadata(&abs_path);

                    // Use EXIF taken_at if available, otherwise fall back to file modified time.
                    // Normalize to consistent YYYY-MM-DDTHH:MM:SS.mmmZ format.
                    let final_taken_at = exif_taken
                        .map(|t| crate::photos::utils::normalize_iso_timestamp(&t))
                        .or(modified);

                    // Compute content-based hash using streaming I/O (avoids loading entire file into memory)
                    let photo_hash = compute_photo_hash_streaming(&abs_path).await;

                    // Use INSERT OR IGNORE to handle race conditions with concurrent
                    // scans (e.g. explicit scan_and_register running simultaneously).
                    let insert_result = sqlx::query(
                        "INSERT OR IGNORE INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                         size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, created_at, photo_hash) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(&photo_id)
                    .bind(&admin_id)
                    .bind(&name)
                    .bind(&rel_path)
                    .bind(&mime)
                    .bind(media_type)
                    .bind(size)
                    .bind(img_w)
                    .bind(img_h)
                    .bind(&final_taken_at)
                    .bind(exif_lat)
                    .bind(exif_lon)
                    .bind(&cam_model)
                    .bind(&thumb_rel)
                    .bind(&now)
                    .bind(&photo_hash)
                    .execute(pool)
                    .await;

                    match insert_result {
                        Ok(result) if result.rows_affected() == 0 => {
                            // Already registered by a concurrent scan — skip
                            tracing::debug!(file = %rel_path, "Already registered (concurrent scan), skipping");
                            continue;
                        }
                        Err(e) => {
                            tracing::error!("Autoscan: failed to register photo {}: {}", rel_path, e);
                            continue;
                        }
                        Ok(_) => { /* inserted successfully */ }
                    }

                    // Generate thumbnail (matches scan_and_register behavior)
                    if has_ffmpeg || mime.starts_with("audio/") {
                        let thumb_abs = storage_root.join(&thumb_rel);
                        if generate_thumbnail_file(&abs_path, &thumb_abs, &mime, None).await {
                            tracing::debug!(file = %rel_path, "Autoscan: generated thumbnail");
                        } else {
                            tracing::warn!(file = %rel_path, "Autoscan: failed to generate thumbnail");
                        }
                    }

                    new_count += 1;
                    tracing::info!(
                        "[DIAG:AUTOSCAN] Registered: {} (type={}, mime={}, size={})",
                        name, media_type, mime, size
                    );
                }
            }
        }
    }

    new_count
}
