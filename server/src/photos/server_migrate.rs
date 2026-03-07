//! Server-side parallel encryption migration.
//!
//! When the encryption mode is switched from "plain" to "encrypted", this module
//! handles the heavy lifting entirely server-side. The client sends the AES-256
//! key once (over HTTPS), and the server reads every plain photo from disk,
//! generates a thumbnail, encrypts both payloads, writes the blobs, and updates
//! the DB — all in parallel across available CPU cores.
//!
//! ## Endpoints
//!
//! - `POST /api/admin/encryption/migrate`  — start the migration (accepts key)
//! - `GET  /api/admin/encryption/migrate/stream` — SSE progress stream

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::blobs::storage;
use crate::crypto;
use crate::error::AppError;
use crate::state::AppState;

use super::scan::{needs_web_preview, generate_web_preview_bg, generate_thumbnail_file};

// ── Request / response types ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StartMigrationRequest {
    /// Hex-encoded AES-256 key (64 hex chars).
    pub key_hex: String,
}

#[derive(Debug, Serialize)]
pub struct StartMigrationResponse {
    pub message: String,
    pub total: i64,
}

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
/// Only one migration can run at a time.
static MIGRATION_PROGRESS: std::sync::OnceLock<tokio::sync::RwLock<Option<Arc<MigrationProgress>>>> =
    std::sync::OnceLock::new();

fn progress_store() -> &'static tokio::sync::RwLock<Option<Arc<MigrationProgress>>> {
    MIGRATION_PROGRESS.get_or_init(|| tokio::sync::RwLock::new(None))
}

// ── Thumbnail generation ────────────────────────────────────────────────────

/// Generate a 256×256 JPEG thumbnail for the migration pipeline.
///
/// Delegates to `scan.rs::generate_thumbnail_file()` which correctly handles:
/// - Video seeking to `-ss 1` (avoids black first frames from fade-ins)
/// - Using the original file path (correct extension for FFmpeg/ImageMagick
///   format detection — fixes `.bin` extension bug for CR2, etc.)
/// - ImageMagick fallback for exotic formats (CR2, SVG, HEIC, CUR)
///
/// For formats the `image` crate handles well (JPEG, PNG, GIF, WebP, BMP,
/// TIFF, ICO, HDR), uses fast in-memory processing as the first attempt.
///
/// Audio files intentionally get a black 256×256 placeholder.
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

    // Non-video: try the image crate first (fast, no subprocess).
    // Works well for JPEG, PNG, GIF, WebP, BMP, TIFF, ICO, HDR.
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
            // A real thumbnail from image crate should be > 1KB.
            // Extremely small output likely means a degenerate decode
            // (e.g. a 1×1 icon from a CUR file).  Fall through to
            // FFmpeg/ImageMagick which may do better.
            if result.len() > 1024 {
                return image_result;
            }
            tracing::debug!(
                "[DIAG:SERVER_MIG] image crate produced small thumb ({} bytes) for {}, trying FFmpeg/ImageMagick",
                result.len(),
                source_path.display()
            );
        }
    }

    // Use generate_thumbnail_file() from scan.rs which:
    //  - Uses the ORIGINAL file path (correct extension for format detection)
    //  - Seeks to -ss 1 for videos (avoids black first-frame problem)
    //  - Falls back to ImageMagick for exotic image formats (CR2, SVG, HEIC, etc.)
    let tmp_output = std::env::temp_dir().join(format!(
        "sp_mig_thumb_{}.jpg",
        Uuid::new_v4()
    ));

    if generate_thumbnail_file(source_path, &tmp_output, mime_type, None).await {
        let result = tokio::fs::read(&tmp_output).await.ok();
        let _ = tokio::fs::remove_file(&tmp_output).await;
        if result.as_ref().map(|d| d.len()).unwrap_or(0) > 0 {
            tracing::debug!(
                "[DIAG:SERVER_MIG] generated thumbnail via scan.rs for {} ({} bytes)",
                source_path.display(),
                result.as_ref().map(|d| d.len()).unwrap_or(0)
            );
            return result;
        }
    }
    let _ = tokio::fs::remove_file(&tmp_output).await;

    tracing::warn!(
        "[DIAG:SERVER_MIG] all thumbnail methods failed for {} (mime={})",
        source_path.display(),
        mime_type
    );
    None
}

// ── Single-photo encryption pipeline ────────────────────────────────────────

/// Row from the `photos` table that needs migration.
#[derive(Debug, sqlx::FromRow)]
struct PlainPhotoRow {
    id: String,
    user_id: String,
    filename: String,
    file_path: String,
    mime_type: String,
    media_type: String,
    size_bytes: i64,
    width: i64,
    height: i64,
    duration_secs: Option<f64>,
    taken_at: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    created_at: String,
}

/// Encrypt one photo: read from disk → web preview (if needed) → thumbnail → encrypt → write blobs → update DB.
async fn encrypt_one_photo(
    photo: PlainPhotoRow,
    key: &[u8; 32],
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
) -> Result<(), String> {
    // Step 1: Read the raw file from disk
    let full_path = storage_root.join(&photo.file_path);
    let file_data = tokio::fs::read(&full_path)
        .await
        .map_err(|e| format!("Read failed for {}: {}", photo.filename, e))?;

    tracing::info!(
        "[DIAG:SERVER_MIG] read file: {} ({} bytes, mime={})",
        photo.filename, file_data.len(), photo.mime_type
    );

    // Step 2: Generate web preview if the format isn't browser-compatible.
    // For videos this means transcoding to MP4, for images HEIC→JPEG, etc.
    // The web-compatible data is what goes into the encrypted blob so the
    // client can display it directly after decryption.
    
    
    // We must ensure web previews are generated BEFORE encryption so the client can play them
    let (payload_data, payload_mime) = if let Some(preview_ext) = needs_web_preview(&photo.filename) {
        let cached_preview_path = storage_root.join(format!(".web_previews/{}.web.{}", photo.id, preview_ext));
        if tokio::fs::try_exists(&cached_preview_path).await.unwrap_or(false) {
            if let Ok(data) = tokio::fs::read(&cached_preview_path).await {
                let preview_mime = match preview_ext {
                    "jpg" => "image/jpeg",
                    "png" => "image/png",
                    "mp3" => "audio/mpeg",
                    "mp4" => "video/mp4",
                    _ => photo.mime_type.as_str(),
                };
                (data, preview_mime.to_string())
            } else {
                (file_data.clone(), photo.mime_type.clone())
            }
        } else {
            tracing::info!("[DIAG:SERVER_MIG] Generating synchronous web preview for {} before encryption", photo.filename);
            let success = generate_web_preview_bg(&full_path, &cached_preview_path, preview_ext).await;
            if success {
                let data = tokio::fs::read(&cached_preview_path).await.unwrap_or(file_data.clone());
                let preview_mime = match preview_ext {
                    "jpg" => "image/jpeg",
                    "png" => "image/png",
                    "mp3" => "audio/mpeg",
                    "mp4" => "video/mp4",
                    _ => photo.mime_type.as_str(),
                };
                (data, preview_mime.to_string())
            } else {
                (file_data.clone(), photo.mime_type.clone())
            }
        }
    } else {
        (file_data.clone(), photo.mime_type.clone())
    };

    // Step 3: Generate thumbnail (async — uses FFmpeg/ImageMagick subprocesses)
    // First, check if a thumbnail was already generated by the background converter.
    // Autoscan uses the pattern `.thumbnails/{id}.thumb.jpg` so check that path.
    let cached_thumb_path = storage_root.join(format!(".thumbnails/{}.thumb.jpg", photo.id));
    let thumb_data = if tokio::fs::try_exists(&cached_thumb_path).await.unwrap_or(false) {
        let cached = tokio::fs::read(&cached_thumb_path).await.ok();
        // Quality check: if cached thumbnail is very small (< 2KB) for a
        // non-audio file, it was likely a bad render (e.g. FFmpeg on SVG).
        // Regenerate instead of using the poor cached version.
        if let Some(ref data) = cached {
            if data.len() < 2048 && !photo.mime_type.starts_with("audio/") {
                tracing::info!(
                    "[DIAG:SERVER_MIG] cached thumbnail for {} is only {} bytes — regenerating for better quality",
                    photo.filename, data.len()
                );
                generate_thumbnail_for_migration(&full_path, &file_data, &photo.mime_type).await
            } else {
                tracing::info!(
                    "[DIAG:SERVER_MIG] using cached thumbnail from .thumbnails for {} ({} bytes)",
                    photo.filename, data.len()
                );
                cached
            }
        } else {
            generate_thumbnail_for_migration(&full_path, &file_data, &photo.mime_type).await
        }
    } else {
        // Generate thumbnail using scan.rs's proven pipeline (correct seeking, file extensions, etc.)
        generate_thumbnail_for_migration(&full_path, &file_data, &photo.mime_type).await
    };

    tracing::debug!(
        "[DIAG:SERVER_MIG] thumbnail: {} → {} bytes",
        photo.filename,
        thumb_data.as_ref().map(|d| d.len()).unwrap_or(0)
    );

    // Step 4: Upload thumbnail blob (if generated)
    let mut thumb_blob_id = String::new();
    if let Some(ref thumb_bytes) = thumb_data {
        let thumb_payload = serde_json::json!({
            "v": 1,
            "photo_blob_id": "",
            "width": 256,
            "height": 256,
            "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, thumb_bytes),
        });
        let thumb_json = serde_json::to_vec(&thumb_payload)
            .map_err(|e| format!("JSON serialize failed: {}", e))?;

        // Encrypt thumbnail (CPU-bound)
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

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(&blob_id)
        .bind(&photo.user_id)
        .bind(blob_type)
        .bind(enc_thumb.len() as i64)
        .bind(&enc_thumb_hash)
        .bind(&now)
        .bind(&blob_storage_path)
        .execute(pool)
        .await
        .map_err(|e| format!("Insert thumbnail blob row: {}", e))?;

        thumb_blob_id = blob_id;
    }

    // Step 5: Build the photo payload JSON (same format as the client).
    // Uses the web-compatible data (transcoded) when available so the
    // client can render it directly after decryption.
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

    // Step 6: Encrypt (CPU-bound, offload to blocking pool)
    let enc_photo = {
        let key_copy = *key;
        let json_clone = photo_json;
        tokio::task::spawn_blocking(move || crypto::encrypt(&key_copy, &json_clone))
            .await
            .map_err(|e| format!("Encrypt task panicked: {}", e))?
            .map_err(|e| format!("Photo encrypt failed: {}", e))?
    };

    let enc_photo_hash = hex::encode(Sha256::digest(&enc_photo));
    // Content hash is still based on the ORIGINAL file for deduplication
    let content_hash = hex::encode(&Sha256::digest(&file_data)[..6]); // 12 hex chars

    // Step 7: Write encrypted blob to disk
    let blob_id = Uuid::new_v4().to_string();
    let blob_storage_path =
        storage::write_blob(storage_root, &photo.user_id, &blob_id, &enc_photo)
            .await
            .map_err(|e| format!("Write photo blob failed: {}", e))?;

    let now = Utc::now().to_rfc3339();
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
    .execute(pool)
    .await
    .map_err(|e| format!("Insert photo blob row: {}", e))?;

    // Step 8: Link the plain photo to its encrypted blob (and thumbnail blob)
    sqlx::query("UPDATE photos SET encrypted_blob_id = ?, encrypted_thumb_blob_id = ? WHERE id = ? AND user_id = ?")
        .bind(&blob_id)
        .bind(if thumb_blob_id.is_empty() { None::<&str> } else { Some(thumb_blob_id.as_str()) })
        .bind(&photo.id)
        .bind(&photo.user_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Mark encrypted failed: {}", e))?;

    Ok(())
}

// ── Parallel migration orchestrator ─────────────────────────────────────────

async fn run_migration(
    key: [u8; 32],
    user_id: String,
    pool: sqlx::SqlitePool,
    storage_root: std::path::PathBuf,
    progress: Arc<MigrationProgress>,
    convert_notify: Arc<tokio::sync::Notify>,
    encryption_key_store: Arc<tokio::sync::RwLock<Option<[u8; 32]>>>,
) {
    // Store the encryption key in AppState so the background converter
    // can decrypt blobs, convert, and re-encrypt independently.
    {
        let mut guard = encryption_key_store.write().await;
        *guard = Some(key);
        tracing::info!("[DIAG:SERVER_MIG] encryption key stored in AppState for background converter");
    }
    // Fetch all plain photos that haven't been encrypted yet
    let photos: Vec<PlainPhotoRow> = match sqlx::query_as::<_, PlainPhotoRow>(
        "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, created_at \
         FROM photos WHERE user_id = ? AND encrypted_blob_id IS NULL \
         ORDER BY created_at ASC",
    )
    .bind(&user_id)
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
        // Mark migration complete in DB
        let _ = sqlx::query(
            "UPDATE encryption_migration SET status = 'idle', completed = total, error = NULL WHERE id = 'singleton'",
        )
        .execute(&pool)
        .await;
        tracing::info!("[DIAG:SERVER_MIG] 0 photos to encrypt, set idle, triggering converter");
        convert_notify.notify_one();
        return;
    }

    // Concurrency: use number of CPU cores, but cap at 8 to avoid
    // overwhelming disk I/O or SQLite write contention.
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
            let _permit = sem.acquire().await.expect("semaphore closed");

            *progress_clone.current_file.write().await = filename.clone();
            let start = std::time::Instant::now();
            tracing::info!("[DIAG:SERVER_MIG] start encrypting: {}", filename);

            match encrypt_one_photo(photo, &key_copy, &pool_clone, &root_clone).await {
                Ok(()) => {
                    let elapsed = start.elapsed();
                    tracing::info!(
                        "[DIAG:SERVER_MIG] done encrypting: {} ({:.2}s)",
                        filename, elapsed.as_secs_f64()
                    );
                    progress_clone.succeeded.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    let elapsed = start.elapsed();
                    tracing::error!(
                        "[DIAG:SERVER_MIG] FAILED encrypting: {} ({:.2}s): {}",
                        filename, elapsed.as_secs_f64(), e
                    );
                    progress_clone.failed.fetch_add(1, Ordering::Relaxed);
                    *progress_clone.last_error.write().await = e;
                }
            }

            let completed = progress_clone.completed.fetch_add(1, Ordering::Relaxed) + 1;

            // Update DB progress every item (so polling clients always see fresh data)
            let total = progress_clone.total.load(Ordering::Relaxed);
            let _ = sqlx::query(
                "UPDATE encryption_migration SET completed = ? WHERE id = 'singleton'",
            )
            .bind(completed as i64)
            .execute(&pool_clone)
            .await;
        });

        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        let _ = handle.await;
    }

    // Finalize
    let wall_time = migration_start.elapsed();
    let succeeded = progress.succeeded.load(Ordering::Relaxed);
    let failed = progress.failed.load(Ordering::Relaxed);
    let completed = progress.completed.load(Ordering::Relaxed);
    let last_error = progress.last_error.read().await.clone();

    tracing::info!(
        "[DIAG:SERVER_MIG] migration wall time: {:.2}s for {} photos ({} parallel workers)",
        wall_time.as_secs_f64(), total, parallelism
    );
    tracing::info!(
        "Server-side migration complete: {}/{} succeeded, {} failed",
        succeeded,
        completed,
        failed
    );

    // ── Repair pass: re-process photos with missing encrypted_thumb_blob_id ──
    // While the encryption key is still available, fix any photos that were
    // encrypted but ended up without a thumbnail blob (e.g., thumbnail
    // generation failed or returned None on the first attempt).
    let missing_thumbs: Vec<PlainPhotoRow> = match sqlx::query_as::<_, PlainPhotoRow>(
        "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, created_at \
         FROM photos WHERE user_id = ? AND encrypted_blob_id IS NOT NULL AND encrypted_thumb_blob_id IS NULL",
    )
    .bind(&user_id)
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("[DIAG:SERVER_MIG] repair pass query failed: {}", e);
            Vec::new()
        }
    };

    if !missing_thumbs.is_empty() {
        tracing::info!(
            "[DIAG:SERVER_MIG] repair pass: {} photo(s) missing encrypted_thumb_blob_id, regenerating",
            missing_thumbs.len()
        );
        for photo in missing_thumbs {
            let filename = photo.filename.clone();
            let photo_id = photo.id.clone();
            let photo_user_id = photo.user_id.clone();
            let full_path = storage_root.join(&photo.file_path);

            // Read the raw file and generate thumbnail using the proven scan.rs pipeline
            let file_data = match tokio::fs::read(&full_path).await {
                Ok(data) => data,
                Err(e) => {
                    tracing::error!("[DIAG:SERVER_MIG] repair: read failed for {}: {}", filename, e);
                    continue;
                }
            };

            let thumb_data = generate_thumbnail_for_migration(&full_path, &file_data, &photo.mime_type).await;

            if let Some(thumb_bytes) = thumb_data {
                // Encrypt and upload thumbnail blob
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
                        tracing::error!("[DIAG:SERVER_MIG] repair: JSON serialize failed for {}: {}", filename, e);
                        continue;
                    }
                };

                let key_copy = key;
                let json_clone = thumb_json;
                let enc_thumb = match tokio::task::spawn_blocking(move || crypto::encrypt(&key_copy, &json_clone)).await {
                    Ok(Ok(data)) => data,
                    Ok(Err(e)) => {
                        tracing::error!("[DIAG:SERVER_MIG] repair: encrypt failed for {}: {}", filename, e);
                        continue;
                    }
                    Err(e) => {
                        tracing::error!("[DIAG:SERVER_MIG] repair: encrypt task panicked for {}: {}", filename, e);
                        continue;
                    }
                };

                let enc_thumb_hash = hex::encode(Sha256::digest(&enc_thumb));
                let blob_id = Uuid::new_v4().to_string();
                let blob_type = if photo.media_type == "video" { "video_thumbnail" } else { "thumbnail" };

                let blob_storage_path = match storage::write_blob(&storage_root, &photo_user_id, &blob_id, &enc_thumb).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("[DIAG:SERVER_MIG] repair: write blob failed for {}: {}", filename, e);
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
                    tracing::error!("[DIAG:SERVER_MIG] repair: insert blob row failed for {}: {}", filename, e);
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
                    tracing::error!("[DIAG:SERVER_MIG] repair: update photos failed for {}: {}", filename, e);
                    continue;
                }

                tracing::info!("[DIAG:SERVER_MIG] repair: generated encrypted thumbnail for {}", filename);
            } else {
                tracing::warn!("[DIAG:SERVER_MIG] repair: thumbnail generation returned None for {}", filename);
            }
        }
    }

    let error_msg = if failed > 0 {
        Some(format!(
            "Migration finished with {}/{} failures. Last: {}",
            failed, completed, last_error
        ))
    } else {
        None
    };

    let _ = sqlx::query(
        "UPDATE encryption_migration SET status = 'idle', completed = total, error = ? WHERE id = 'singleton'",
    )
    .bind(&error_msg)
    .execute(&pool)
    .await;

    tracing::info!(
        "[DIAG:SERVER_MIG] migration finalized, set idle, triggering converter in 5s"
    );

    // 5-second delay before triggering the converter — ensures all DB writes
    // have fully settled and gives the system a brief breather.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    convert_notify.notify_one();

    // Keep the encryption key available for a grace period so the background
    // converter can process any encrypted blobs that need conversion.
    // After 30 minutes, clear the key from memory for security.
    tracing::info!(
        "[DIAG:SERVER_MIG] keeping encryption key in memory for 30 min post-migration conversion"
    );
    let key_store_clone = encryption_key_store.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30 * 60)).await;
        let mut guard = key_store_clone.write().await;
        *guard = None;
        tracing::info!("[DIAG:SERVER_MIG] encryption key cleared from memory (30 min grace period expired)");
    });

    progress.running.store(false, Ordering::Release);
}

// ── HTTP handlers ───────────────────────────────────────────────────────────

/// POST /api/admin/encryption/migrate
///
/// Starts a server-side encryption migration. The client sends the AES-256 key
/// (hex-encoded, 64 chars) over HTTPS. The server does all the work locally.
pub async fn start_migration(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<StartMigrationRequest>,
) -> Result<Json<StartMigrationResponse>, AppError> {
    // Admin check
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    // Parse & validate key
    let key = crypto::parse_key_hex(&req.key_hex)
        .map_err(|e| AppError::BadRequest(e))?;

    // Check that encryption mode is "encrypted" and migration is in progress
    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "idle".to_string());

    if mig_status != "encrypting" {
        return Err(AppError::BadRequest(
            "No active encryption migration. Set encryption mode to 'encrypted' first.".into(),
        ));
    }

    // Check if a migration is already running in-process
    {
        let guard = progress_store().read().await;
        if let Some(ref p) = *guard {
            if p.running.load(Ordering::Acquire) {
                return Err(AppError::Conflict(
                    "A server-side migration is already running.".into(),
                ));
            }
        }
    }

    // Count items to migrate
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE user_id = ? AND encrypted_blob_id IS NULL",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    if count == 0 {
        // Nothing to do — mark idle
        sqlx::query(
            "UPDATE encryption_migration SET status = 'idle', completed = total, error = NULL WHERE id = 'singleton'",
        )
        .execute(&state.pool)
        .await?;
        return Ok(Json(StartMigrationResponse {
            message: "No photos to migrate.".into(),
            total: 0,
        }));
    }

    // Set up progress tracker
    let progress = Arc::new(MigrationProgress::new(count as u64));
    {
        let mut guard = progress_store().write().await;
        *guard = Some(progress.clone());
    }

    // Spawn the migration in the background
    let pool = state.pool.clone();
    let storage_root = state.storage_root.read().await.clone();
    let user_id = auth.user_id.clone();
    let convert_notify = state.convert_notify.clone();
    let encryption_key_store = state.encryption_key.clone();
    tokio::spawn(async move {
        run_migration(key, user_id, pool, storage_root, progress, convert_notify, encryption_key_store).await;
    });

    Ok(Json(StartMigrationResponse {
        message: format!("Server-side migration started for {} photos.", count),
        total: count,
    }))
}

/// GET /api/admin/encryption/migrate/stream
///
/// Server-Sent Events stream that pushes migration progress every 500ms.
/// Format: `data: {"completed":N,"total":N,"succeeded":N,"failed":N,"current_file":"...","done":bool}\n\n`
pub async fn migration_stream(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Response, AppError> {
    // Admin check
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let progress = {
        let guard = progress_store().read().await;
        guard.clone()
    };

    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            interval.tick().await;

            let (completed, total, succeeded, failed, running, current_file, last_error) =
                if let Some(ref p) = progress {
                    (
                        p.completed.load(Ordering::Relaxed),
                        p.total.load(Ordering::Relaxed),
                        p.succeeded.load(Ordering::Relaxed),
                        p.failed.load(Ordering::Relaxed),
                        p.running.load(Ordering::Acquire),
                        p.current_file.read().await.clone(),
                        p.last_error.read().await.clone(),
                    )
                } else {
                    // No migration running — send a "done" event and stop
                    let msg = serde_json::json!({
                        "completed": 0, "total": 0, "succeeded": 0, "failed": 0,
                        "current_file": "", "done": true, "last_error": "",
                    });
                    yield Ok::<_, std::convert::Infallible>(
                        format!("data: {}\n\n", msg)
                    );
                    break;
                };

            let done = !running;
            let msg = serde_json::json!({
                "completed": completed,
                "total": total,
                "succeeded": succeeded,
                "failed": failed,
                "current_file": current_file,
                "done": done,
                "last_error": last_error,
            });

            yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", msg));

            if done {
                break;
            }
        }
    };

    let body = Body::from_stream(stream);
    Ok(Response::builder()
        .header("Content-Type", HeaderValue::from_static("text/event-stream"))
        .header("Cache-Control", HeaderValue::from_static("no-cache"))
        .header("Connection", HeaderValue::from_static("keep-alive"))
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}
