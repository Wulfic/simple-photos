//! Server-side parallel encryption migration.
//!
//! On startup (and after each autoscan), if any photos have
//! `encrypted_blob_id IS NULL`, this module auto-migrates them: reads the
//! plain file from disk, generates a thumbnail, encrypts both payloads with
//! AES-256-GCM, writes the blobs, and updates the DB — all in parallel.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::blobs::storage;
use crate::crypto;

use super::scan::{generate_thumbnail_file, generate_web_preview_bg, needs_web_preview};

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

// ── Thumbnail generation ────────────────────────────────────────────────────

/// Generate a 256×256 JPEG thumbnail for the migration pipeline.
///
/// Multi-stage fallback:
/// 1. Audio → black 256×256 placeholder.
/// 2. Non-video images → `image::load_from_memory` first (fast, in-memory).
/// 3. Fallback → `generate_thumbnail_file` from scan.rs (FFmpeg/ImageMagick).
async fn generate_thumbnail_for_migration(
    source_path: &std::path::Path,
    data: &[u8],
    mime_type: &str,
) -> Option<Vec<u8>> {
    // Audio: intentional black placeholder
    if mime_type.starts_with("audio/") {
        return tokio::task::spawn_blocking(|| {
            let img = image::RgbImage::from_pixel(256, 256, image::Rgb([0u8, 0, 0]));
            let mut buf = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(img)
                .write_to(&mut buf, image::ImageFormat::Jpeg)
                .ok()?;
            Some(buf.into_inner())
        })
        .await
        .ok()
        .flatten();
    }

    // Non-video: try the image crate first (fast, no subprocess)
    if !mime_type.starts_with("video/") {
        let data_owned = data.to_vec();
        let image_result = tokio::task::spawn_blocking(move || {
            if let Ok(img) = image::load_from_memory(&data_owned) {
                let thumb = img.resize_to_fill(256, 256, image::imageops::FilterType::Triangle);
                let mut buf = std::io::Cursor::new(Vec::new());
                if thumb
                    .write_to(&mut buf, image::ImageFormat::Jpeg)
                    .is_ok()
                {
                    return Some(buf.into_inner());
                }
            }
            None
        })
        .await
        .ok()
        .flatten();

        if let Some(ref result) = image_result {
            if result.len() > 1024 {
                return image_result;
            }
            tracing::debug!(
                "[SERVER_MIG] image crate produced small thumb ({} bytes) for {}, trying FFmpeg",
                result.len(),
                source_path.display()
            );
        }
    }

    // Fallback: generate_thumbnail_file from scan.rs
    let tmp_output = std::env::temp_dir().join(format!("sp_mig_thumb_{}.jpg", Uuid::new_v4()));

    if generate_thumbnail_file(source_path, &tmp_output, mime_type, None).await {
        let result = tokio::fs::read(&tmp_output).await.ok();
        let _ = tokio::fs::remove_file(&tmp_output).await;
        if result.as_ref().map(|d| d.len()).unwrap_or(0) > 0 {
            return result;
        }
    }
    let _ = tokio::fs::remove_file(&tmp_output).await;

    tracing::warn!(
        "[SERVER_MIG] all thumbnail methods failed for {} (mime={})",
        source_path.display(),
        mime_type
    );
    None
}

// ── Single-photo encryption pipeline ────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct PlainPhotoRow {
    id: String,
    user_id: String,
    filename: String,
    file_path: String,
    mime_type: String,
    media_type: String,
    #[allow(dead_code)]
    size_bytes: i64,
    width: i64,
    height: i64,
    duration_secs: Option<f64>,
    taken_at: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    created_at: String,
}

/// Encrypt one photo: read from disk → web preview → thumbnail → encrypt → write blobs → update DB.
async fn encrypt_one_photo(
    photo: PlainPhotoRow,
    key: &[u8; 32],
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
) -> Result<(), String> {
    let full_path = storage_root.join(&photo.file_path);
    let file_data = tokio::fs::read(&full_path)
        .await
        .map_err(|e| format!("Read failed for {}: {}", photo.filename, e))?;

    tracing::info!(
        "[SERVER_MIG] read file: {} ({} bytes, mime={})",
        photo.filename,
        file_data.len(),
        photo.mime_type
    );

    // Generate web preview and thumbnail concurrently
    let web_preview_fut = async {
        if let Some(preview_ext) = needs_web_preview(&photo.filename) {
            let cached_preview_path = storage_root.join(format!(
                ".web_previews/{}.web.{}",
                photo.id, preview_ext
            ));
            if tokio::fs::try_exists(&cached_preview_path)
                .await
                .unwrap_or(false)
            {
                if let Ok(data) = tokio::fs::read(&cached_preview_path).await {
                    let preview_mime = match preview_ext {
                        "jpg" => "image/jpeg",
                        "png" => "image/png",
                        "mp3" => "audio/mpeg",
                        "mp4" => "video/mp4",
                        _ => photo.mime_type.as_str(),
                    };
                    return (data, preview_mime.to_string());
                }
            } else {
                tracing::info!(
                    "[SERVER_MIG] Generating web preview for {} before encryption",
                    photo.filename
                );
                let success =
                    generate_web_preview_bg(&full_path, &cached_preview_path, preview_ext).await;
                if success {
                    let data = tokio::fs::read(&cached_preview_path)
                        .await
                        .unwrap_or_else(|_| file_data.clone());
                    let preview_mime = match preview_ext {
                        "jpg" => "image/jpeg",
                        "png" => "image/png",
                        "mp3" => "audio/mpeg",
                        "mp4" => "video/mp4",
                        _ => photo.mime_type.as_str(),
                    };
                    return (data, preview_mime.to_string());
                }
            }
        }
        (file_data.clone(), photo.mime_type.clone())
    };

    let thumbnail_fut = async {
        let cached_thumb_path =
            storage_root.join(format!(".thumbnails/{}.thumb.jpg", photo.id));
        if tokio::fs::try_exists(&cached_thumb_path)
            .await
            .unwrap_or(false)
        {
            let cached = tokio::fs::read(&cached_thumb_path).await.ok();
            if let Some(ref data) = cached {
                if data.len() < 2048 && !photo.mime_type.starts_with("audio/") {
                    tracing::info!(
                        "[SERVER_MIG] cached thumbnail for {} is only {} bytes — regenerating",
                        photo.filename,
                        data.len()
                    );
                    return generate_thumbnail_for_migration(
                        &full_path,
                        &file_data,
                        &photo.mime_type,
                    )
                    .await;
                }
                return cached;
            }
            return generate_thumbnail_for_migration(&full_path, &file_data, &photo.mime_type)
                .await;
        }
        generate_thumbnail_for_migration(&full_path, &file_data, &photo.mime_type).await
    };

    let ((payload_data, payload_mime), thumb_data) =
        tokio::join!(web_preview_fut, thumbnail_fut);

    // Upload thumbnail blob (if generated)
    let mut thumb_blob_id = String::new();
    let thumb_insert_params = if let Some(ref thumb_bytes) = thumb_data {
        let thumb_payload = serde_json::json!({
            "v": 1,
            "photo_blob_id": "",
            "width": 256,
            "height": 256,
            "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, thumb_bytes),
        });
        let thumb_json = serde_json::to_vec(&thumb_payload)
            .map_err(|e| format!("JSON serialize failed: {}", e))?;

        let enc_thumb = {
            let key_copy = *key;
            let json_clone = thumb_json;
            tokio::task::spawn_blocking(move || crypto::encrypt(&key_copy, &json_clone))
                .await
                .map_err(|e| format!("Encrypt task panicked: {}", e))?
                .map_err(|e| format!("Thumbnail encrypt failed: {}", e))?
        };

        let enc_thumb_hash = hex::encode(Sha256::digest(&enc_thumb));
        let blob_id = Uuid::new_v4().to_string();
        let blob_type = if photo.media_type == "video" {
            "video_thumbnail"
        } else {
            "thumbnail"
        };

        let blob_storage_path =
            storage::write_blob(storage_root, &photo.user_id, &blob_id, &enc_thumb)
                .await
                .map_err(|e| format!("Write thumbnail blob failed: {}", e))?;

        let thumb_now = Utc::now().to_rfc3339();
        let params = Some((
            blob_id.clone(),
            blob_type.to_string(),
            enc_thumb.len() as i64,
            enc_thumb_hash,
            thumb_now,
            blob_storage_path,
        ));

        thumb_blob_id = blob_id;
        params
    } else {
        None
    };

    // Build the photo payload JSON (same format as the client)
    let server_blob_type = if photo.mime_type == "image/gif" {
        "gif"
    } else if photo.mime_type.starts_with("video/") {
        "video"
    } else if photo.mime_type.starts_with("audio/") {
        "audio"
    } else {
        "photo"
    };

    let photo_payload = serde_json::json!({
        "v": 1,
        "filename": photo.filename,
        "taken_at": photo.taken_at.as_deref().unwrap_or(&photo.created_at),
        "mime_type": payload_mime,
        "media_type": photo.media_type,
        "width": photo.width,
        "height": photo.height,
        "duration": photo.duration_secs,
        "latitude": photo.latitude,
        "longitude": photo.longitude,
        "album_ids": [],
        "thumbnail_blob_id": thumb_blob_id,
        "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &payload_data),
    });
    let photo_json = serde_json::to_vec(&photo_payload)
        .map_err(|e| format!("JSON serialize failed: {}", e))?;

    // Encrypt (CPU-bound, offload to blocking pool)
    let enc_photo = {
        let key_copy = *key;
        let json_clone = photo_json;
        tokio::task::spawn_blocking(move || crypto::encrypt(&key_copy, &json_clone))
            .await
            .map_err(|e| format!("Encrypt task panicked: {}", e))?
            .map_err(|e| format!("Photo encrypt failed: {}", e))?
    };

    let enc_photo_hash = hex::encode(Sha256::digest(&enc_photo));
    let content_hash = hex::encode(&Sha256::digest(&file_data)[..6]);

    // Write encrypted blob to disk
    let blob_id = Uuid::new_v4().to_string();
    let blob_storage_path =
        storage::write_blob(storage_root, &photo.user_id, &blob_id, &enc_photo)
            .await
            .map_err(|e| format!("Write photo blob failed: {}", e))?;

    let now = Utc::now().to_rfc3339();

    // Atomic transaction: INSERT blobs + UPDATE photos
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("Begin tx: {}", e))?;

    if let Some((ref tid, ref ttype, tsize, ref thash, ref ttime, ref tpath)) = thumb_insert_params
    {
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(tid)
        .bind(&photo.user_id)
        .bind(ttype)
        .bind(tsize)
        .bind(thash)
        .bind(ttime)
        .bind(tpath)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Insert thumbnail blob row: {}", e))?;
    }

    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&blob_id)
    .bind(&photo.user_id)
    .bind(server_blob_type)
    .bind(enc_photo.len() as i64)
    .bind(&enc_photo_hash)
    .bind(&now)
    .bind(&blob_storage_path)
    .bind(&content_hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Insert photo blob row: {}", e))?;

    sqlx::query(
        "UPDATE photos SET encrypted_blob_id = ?, encrypted_thumb_blob_id = ? WHERE id = ? AND user_id = ?",
    )
    .bind(&blob_id)
    .bind(if thumb_blob_id.is_empty() {
        None::<&str>
    } else {
        Some(thumb_blob_id.as_str())
    })
    .bind(&photo.id)
    .bind(&photo.user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Mark encrypted failed: {}", e))?;

    tx.commit()
        .await
        .map_err(|e| format!("Commit tx: {}", e))?;

    Ok(())
}

// ── Parallel migration orchestrator ─────────────────────────────────────────

async fn run_migration(
    key: [u8; 32],
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
    progress: Arc<MigrationProgress>,
) {
    // Check audio backup toggle
    let audio_backup_enabled: bool = sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(true);

    // Fetch all photos that haven't been encrypted yet
    let all_photos: Vec<PlainPhotoRow> = match sqlx::query_as::<_, PlainPhotoRow>(
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

    // Filter out audio files if toggle is off
    let photos: Vec<PlainPhotoRow> = if audio_backup_enabled {
        all_photos
    } else {
        let before = all_photos.len();
        let filtered: Vec<_> = all_photos
            .into_iter()
            .filter(|p| p.media_type != "audio")
            .collect();
        let skipped = before - filtered.len();
        if skipped > 0 {
            tracing::info!(
                "[SERVER_MIG] skipped {} audio files (audio backup disabled)",
                skipped
            );
        }
        filtered
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

    // ── Repair pass: fix photos with missing encrypted_thumb_blob_id ──
    let missing_thumbs: Vec<PlainPhotoRow> = match sqlx::query_as::<_, PlainPhotoRow>(
        "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, created_at \
         FROM photos WHERE encrypted_blob_id IS NOT NULL AND encrypted_thumb_blob_id IS NULL",
    )
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("[SERVER_MIG] repair pass query failed: {}", e);
            Vec::new()
        }
    };

    if !missing_thumbs.is_empty() {
        tracing::info!(
            "[SERVER_MIG] repair pass: {} photo(s) missing thumbnails",
            missing_thumbs.len()
        );
        for photo in missing_thumbs {
            let filename = photo.filename.clone();
            let photo_id = photo.id.clone();
            let photo_user_id = photo.user_id.clone();
            let full_path = storage_root.join(&photo.file_path);

            let file_data = match tokio::fs::read(&full_path).await {
                Ok(data) => data,
                Err(e) => {
                    tracing::error!(
                        "[SERVER_MIG] repair: read failed for {}: {}",
                        filename,
                        e
                    );
                    continue;
                }
            };

            let thumb_data =
                generate_thumbnail_for_migration(&full_path, &file_data, &photo.mime_type).await;

            if let Some(thumb_bytes) = thumb_data {
                let thumb_payload = serde_json::json!({
                    "v": 1,
                    "photo_blob_id": "",
                    "width": 256,
                    "height": 256,
                    "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &thumb_bytes),
                });
                let thumb_json = match serde_json::to_vec(&thumb_payload) {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::error!(
                            "[SERVER_MIG] repair: JSON serialize failed for {}: {}",
                            filename,
                            e
                        );
                        continue;
                    }
                };

                let key_copy = key;
                let json_clone = thumb_json;
                let enc_thumb = match tokio::task::spawn_blocking(move || {
                    crypto::encrypt(&key_copy, &json_clone)
                })
                .await
                {
                    Ok(Ok(data)) => data,
                    Ok(Err(e)) => {
                        tracing::error!(
                            "[SERVER_MIG] repair: encrypt failed for {}: {}",
                            filename,
                            e
                        );
                        continue;
                    }
                    Err(e) => {
                        tracing::error!(
                            "[SERVER_MIG] repair: task panicked for {}: {}",
                            filename,
                            e
                        );
                        continue;
                    }
                };

                let enc_thumb_hash = hex::encode(Sha256::digest(&enc_thumb));
                let blob_id = Uuid::new_v4().to_string();
                let blob_type = if photo.media_type == "video" {
                    "video_thumbnail"
                } else {
                    "thumbnail"
                };

                let blob_storage_path = match storage::write_blob(
                    &storage_root,
                    &photo_user_id,
                    &blob_id,
                    &enc_thumb,
                )
                .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(
                            "[SERVER_MIG] repair: write blob failed for {}: {}",
                            filename,
                            e
                        );
                        continue;
                    }
                };

                let now = Utc::now().to_rfc3339();
                if let Err(e) = sqlx::query(
                    "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
                )
                .bind(&blob_id)
                .bind(&photo_user_id)
                .bind(blob_type)
                .bind(enc_thumb.len() as i64)
                .bind(&enc_thumb_hash)
                .bind(&now)
                .bind(&blob_storage_path)
                .execute(&pool)
                .await
                {
                    tracing::error!(
                        "[SERVER_MIG] repair: insert blob failed for {}: {}",
                        filename,
                        e
                    );
                    continue;
                }

                if let Err(e) = sqlx::query(
                    "UPDATE photos SET encrypted_thumb_blob_id = ? WHERE id = ? AND user_id = ?",
                )
                .bind(&blob_id)
                .bind(&photo_id)
                .bind(&photo_user_id)
                .execute(&pool)
                .await
                {
                    tracing::error!(
                        "[SERVER_MIG] repair: update photos failed for {}: {}",
                        filename,
                        e
                    );
                    continue;
                }

                tracing::info!(
                    "[SERVER_MIG] repair: generated encrypted thumbnail for {}",
                    filename
                );
            }
        }
    }

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

/// Start encryption migration for all unencrypted photos.
/// Called after autoscan and on startup.
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

    let count: i64 = match sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    {
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

    run_migration(key, pool, storage_root, progress).await;
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

    let unencrypted_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

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
    let unencrypted_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

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
