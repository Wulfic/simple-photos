//! Automatic filesystem scanner that registers new **native** media files
//! into the database.
//!
//! Runs as a background task on a configurable interval and can also be
//! triggered on-demand via `POST /api/admin/photos/auto-scan`.  Files are
//! assigned to the first admin user; duplicates are handled gracefully with
//! `INSERT OR IGNORE` to avoid race conditions with concurrent scans.
//!
//! Only browser-native formats are handled here.  After native files are
//! imported and encrypted, the ingest engine ([`crate::ingest`]) runs a
//! separate conversion pass for non-native formats.

use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use axum::extract::State;
use axum::Json;
use futures_util::TryStreamExt;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::media::{is_media_file, mime_from_extension};
use crate::photos::metadata::extract_media_metadata_async;
use crate::photos::thumbnail::generate_thumbnail_file;
use crate::photos::utils::compute_photo_hash_streaming;
use crate::state::AppState;

/// Background task: automatically scan the storage directory for new files
/// on a configurable interval (or when triggered by an API call).
///
/// Reads the **current** storage root from `ArcSwap` on every iteration so
/// that runtime storage-path changes (via the setup wizard or admin API) are
/// picked up immediately — no server restart required.
pub async fn background_auto_scan_task(
    pool: sqlx::SqlitePool,
    storage_root: Arc<ArcSwap<PathBuf>>,
    interval_secs: u64,
    scan_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    jwt_secret: String,
) {
    if interval_secs == 0 {
        tracing::info!("Background auto-scan disabled (interval = 0)");
        return;
    }

    // Run an initial scan shortly after startup
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    tracing::info!("[DIAG:AUTOSCAN] Running startup auto-scan...");
    let root = (**storage_root.load()).clone();
    let count = if let Ok(_guard) = scan_lock.try_lock() {
        run_auto_scan(&pool, &root).await
    } else {
        tracing::info!("[DIAG:AUTOSCAN] Startup scan skipped — another scan is in progress");
        0
    };
    tracing::info!(
        "[DIAG:AUTOSCAN] Startup auto-scan complete: registered {} new files",
        count
    );
    update_last_scan_time(&pool).await;

    if count > 0 {
        crate::audit::log_background(
            &pool,
            crate::audit::AuditEvent::AutoScanComplete,
            Some(serde_json::json!({"trigger": "startup", "new_count": count})),
        );
    }

    // After startup scan, trigger encryption then conversion ingest engine.
    // Sequencing: native encrypt FIRST → conversion → encrypt converted.
    {
        let pool_clone = pool.clone();
        let root_clone = root.clone();
        let jwt_clone = jwt_secret.clone();
        tokio::spawn(async move {
            if count > 0 {
                crate::photos::server_migrate::auto_migrate_after_scan(
                    pool_clone.clone(), root_clone.clone(), jwt_clone.clone(),
                ).await;
            }
            crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_clone).await;
        });
    }

    // Then scan on a configurable interval
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    tracing::info!("Auto-scan interval: every {} seconds", interval_secs);

    loop {
        interval.tick().await;
        let root = (**storage_root.load()).clone();
        let count = if let Ok(_guard) = scan_lock.try_lock() {
            run_auto_scan(&pool, &root).await
        } else {
            tracing::info!("[DIAG:AUTOSCAN] Interval scan skipped — another scan is in progress");
            0
        };
        tracing::info!(
            "[DIAG:AUTOSCAN] Interval auto-scan complete: registered {} new files",
            count
        );
        update_last_scan_time(&pool).await;

        if count > 0 {
            crate::audit::log_background(
                &pool,
                crate::audit::AuditEvent::AutoScanComplete,
                Some(serde_json::json!({"trigger": "interval", "new_count": count})),
            );
        }

        // Trigger encryption then conversion ingest engine.
        {
            let pool_clone = pool.clone();
            let root_clone = root.clone();
            let jwt_clone = jwt_secret.clone();
            tokio::spawn(async move {
                if count > 0 {
                    crate::photos::server_migrate::auto_migrate_after_scan(
                        pool_clone.clone(), root_clone.clone(), jwt_clone.clone(),
                    ).await;
                }
                crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_clone).await;
            });
        }
    }
}

async fn update_last_scan_time(pool: &sqlx::SqlitePool) {
    let now = crate::photos::utils::utc_now_iso();
    if let Err(e) = sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('last_auto_scan', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&now)
    .execute(pool)
    .await
    {
        tracing::warn!("Failed to update last_auto_scan timestamp: {}", e);
    }
}

/// POST /api/admin/photos/auto-scan
/// Trigger an immediate auto-scan (called when web UI or app opens).
/// Runs synchronously so the client can await completion before loading photos.
/// Admin only — the route is under `/api/admin/`.
pub async fn trigger_auto_scan(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    crate::setup::admin::require_admin(&state, &auth).await?;

    // Serialize with other scan operations (manual scan, background autoscan).
    let _scan_guard = state.scan_lock.lock().await;

    let pool = state.pool.clone();
    let storage_root = (**state.storage_root.load()).clone();

    let count = run_auto_scan(&pool, &storage_root).await;
    tracing::info!(
        "[DIAG:AUTOSCAN] On-demand scan complete: registered {} new files",
        count
    );

    // Update last scan time
    update_last_scan_time(&pool).await;

    crate::audit::log(
        &state,
        crate::audit::AuditEvent::AutoScanComplete,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "trigger": "manual",
            "new_count": count,
        })),
    )
    .await;

    // Trigger encryption then conversion ingest engine.
    {
        let pool_clone = pool.clone();
        let root_clone = storage_root.clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            if count > 0 {
                crate::photos::server_migrate::auto_migrate_after_scan(
                    pool_clone.clone(), root_clone.clone(), jwt_secret.clone(),
                ).await;
            }
            crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_secret).await;
        });
    }

    Ok(Json(serde_json::json!({
        "message": "Scan complete",
        "new_count": count,
    })))
}

/// Scan storage directory and register any unregistered media files for ALL users.
///
/// Public alias so other modules (e.g. encryption key storage) can trigger a
/// scan without going through the HTTP handler.
pub async fn run_auto_scan_public(pool: &sqlx::SqlitePool, storage_root: &std::path::Path) -> i64 {
    run_auto_scan(pool, storage_root).await
}

/// Scan storage directory and register any unregistered media files for ALL users.
async fn run_auto_scan(pool: &sqlx::SqlitePool, storage_root: &std::path::Path) -> i64 {
    // Skip scanning while a disaster-recovery push is in-flight to avoid
    // creating duplicate photo rows that race with the incoming sync.
    let recovering: bool = sqlx::query_scalar(
        "SELECT value = 'true' FROM server_settings WHERE key = 'recovery_in_progress'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(false);
    if recovering {
        tracing::info!("[DIAG:AUTOSCAN] run_auto_scan: recovery in progress, skipping");
        return 0;
    }

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

    // Check audio-backup toggle — skip audio files unless enabled.
    let audio_enabled: bool = sqlx::query_scalar(
        "SELECT value = 'true' FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(false);

    // Build set of already-registered paths (from both active photos and trash)
    // using a streaming cursor so we never hold the full Vec<String> + HashSet
    // simultaneously in memory.
    let mut existing_set = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM photos WHERE file_path != '' \
             UNION SELECT source_path FROM photos WHERE source_path IS NOT NULL AND source_path != '' \
             UNION SELECT file_path FROM trash_items WHERE file_path != ''"
        ).fetch(pool);

        while let Some(path) = rows.try_next().await.unwrap_or(None) {
            existing_set.insert(path);
        }
    }
    tracing::info!(
        "[DIAG:AUTOSCAN] run_auto_scan: {} existing photos in DB, scanning {:?}",
        existing_set.len(),
        storage_root
    );

    // Build set of content hashes belonging to gallery-hidden originals.
    // After recovery, the photos table doesn't have rows for these (excluded
    // from sync_photos), but their content hashes are stored in the egi table.
    // Any file on disk whose hash matches should NOT be registered — it belongs
    // to a secure gallery item and must stay hidden.
    let mut gallery_hashes = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT original_photo_hash FROM encrypted_gallery_items WHERE original_photo_hash IS NOT NULL"
        ).fetch(pool);

        while let Some(hash) = rows.try_next().await.unwrap_or(None) {
            gallery_hashes.insert(hash);
        }
    }
    if !gallery_hashes.is_empty() {
        tracing::info!(
            "[DIAG:AUTOSCAN] run_auto_scan: {} gallery-hidden hashes to exclude",
            gallery_hashes.len()
        );
    }

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

                    // Native format — determine MIME and media type directly.
                    let mime = mime_from_extension(&name).to_string();
                    let media_type: &str = if mime.starts_with("video/") {
                        "video"
                    } else if mime.starts_with("audio/") {
                        "audio"
                    } else if mime == "image/gif" {
                        "gif"
                    } else {
                        "photo"
                    };

                    if media_type == "audio" && !audio_enabled {
                        continue;
                    }

                    let photo_id = Uuid::new_v4().to_string();
                    let now = crate::photos::utils::utc_now_iso();
                    // Use .thumb.gif for GIFs so the thumbnail preserves animation
                    let thumb_ext = if mime == "image/gif" { "gif" } else { "jpg" };
                    let thumb_rel = format!(".thumbnails/{}.thumb.{}", photo_id, thumb_ext);

                    // Extract dimensions, camera model, GPS, and date from file
                    let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
                        extract_media_metadata_async(abs_path.clone()).await;

                    let final_taken_at = exif_taken
                        .map(|t| crate::photos::utils::normalize_iso_timestamp(&t))
                        .or(modified);

                    let photo_hash = compute_photo_hash_streaming(&abs_path).await;

                    // Skip files whose content hash matches a gallery-hidden original.
                    if let Some(ref h) = photo_hash {
                        if gallery_hashes.contains(h) {
                            tracing::info!(
                                "[DIAG:AUTOSCAN] Skipping {} — content hash {} matches gallery-hidden original",
                                rel_path, h
                            );
                            continue;
                        }
                    }

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
                            tracing::debug!(file = %rel_path, "Already registered (concurrent scan), skipping");
                            continue;
                        }
                        Err(e) => {
                            tracing::error!(
                                "Autoscan: failed to register photo {}: {}",
                                rel_path,
                                e
                            );
                            continue;
                        }
                        Ok(_) => { /* inserted successfully */ }
                    }

                    // Generate thumbnail
                    {
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
                        name,
                        media_type,
                        mime,
                        size
                    );
                }
            }
        }
    }

    new_count
}
