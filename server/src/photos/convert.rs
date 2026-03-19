//! Background media processing pipeline (three-phase sequential design).
//!
//! ## Processing Order
//!
//! Every cycle runs three strictly-ordered phases so that the gallery always
//! shows thumbnails as early as possible:
//!
//! 1. **Phase 1 — Thumbnails**: Generate thumbnails for ALL files that are
//!    missing them.  This includes both files that need no conversion (native
//!    browser formats) and source files that *do* need conversion but whose
//!    original can still produce a decent thumbnail (e.g. MKV frame-extract).
//!
//! 2. **Phase 2 — Conversion**: Transcode / convert every flagged file to a
//!    browser/Android-friendly format (HEIC → JPEG, MKV → MP4, AIFF → MP3,
//!    etc.) using low-priority FFmpeg (`nice -n 19`).  Original files are
//!    **never deleted** — the converted copy lives in `.web_previews/` and
//!    the original remains available via `GET /photos/:id/file`.
//!
//! 3. **Phase 3 — Post-Conversion Thumbnails**: For files that were just
//!    converted in Phase 2, regenerate their thumbnail from the new
//!    web-compatible preview if the existing thumbnail is missing or of poor
//!    quality.  Only freshly-converted files are touched.
//!
//! ## Triggers
//!
//! The pipeline runs on a 60-second timer **and** can be woken immediately
//! via a `Notify` handle (e.g. after a scan, upload, or migration completes).
//!
//! ## Encrypted photo support
//!
//! When the encryption key is available in `AppState` (stored temporarily
//! during and after migration), the converter can decrypt encrypted blobs,
//! convert the media, and re-encrypt with the web-compatible data.  This
//! makes conversion independent of encryption.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tokio::sync::{Notify, RwLock, Semaphore};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::blobs::storage;
use crate::crypto;
use crate::error::AppError;
use crate::setup::admin::require_admin;
use crate::state::AppState;

use super::scan::{needs_web_preview, generate_web_preview_bg, generate_thumbnail_file};

/// Row type returned by the photos query shared across all three phases.
type PhotoRow = (String, String, String, Option<String>, String, Option<String>, String);

/// Maximum concurrent FFmpeg/ImageMagick subprocesses for thumbnail and
/// conversion phases.  Bounded to avoid fork-bombing the system.
const MEDIA_PARALLELISM: usize = 4;

/// Cached FFmpeg availability flag.  Checked once per process lifetime
/// to avoid spawning `ffmpeg -version` on every 60-second cycle.
static FFMPEG_AVAILABLE: std::sync::OnceLock<AtomicBool> = std::sync::OnceLock::new();

async fn ffmpeg_available_cached() -> bool {
    let flag = FFMPEG_AVAILABLE.get_or_init(|| {
        // Initialise as false; first call to check_and_cache will set true.
        AtomicBool::new(false)
    });
    // Fast path: already cached.
    if flag.load(Ordering::Relaxed) {
        return true;
    }
    // Slow path: run the actual check once.
    let available = super::scan::ffmpeg_available_pub().await;
    if available {
        flag.store(true, Ordering::Relaxed);
    }
    available
}

// ── Phase 1: Generate ALL missing thumbnails ────────────────────────────────

/// Generate missing thumbnails for every photo.  This runs first so the
/// gallery shows something useful before the (potentially slow) conversion
/// phase begins.  Encrypted photos are skipped — their thumbnails are
/// encrypted blobs created during migration.
async fn phase_thumbnails(
    photos: &[PhotoRow],
    storage_root: &PathBuf,
) -> u32 {
    // Collect work items first (cheap fs::try_exists checks), then run
    // actual thumbnail generation in parallel bounded by MEDIA_PARALLELISM.
    let mut work: Vec<(String, PathBuf, PathBuf, String)> = Vec::new();
    for (photo_id, file_path, _filename, thumb_path, mime_type, encrypted_blob_id, _user_id) in photos {
        if encrypted_blob_id.is_some() {
            continue;
        }
        let tp = match thumb_path {
            Some(tp) => tp,
            None => continue,
        };
        let thumb_abs = storage_root.join(tp);
        if tokio::fs::try_exists(&thumb_abs).await.unwrap_or(false) {
            continue;
        }
        let source_path = storage_root.join(file_path);
        if !tokio::fs::try_exists(&source_path).await.unwrap_or(false) {
            continue;
        }
        work.push((photo_id.clone(), source_path, thumb_abs, mime_type.clone()));
    }

    if work.is_empty() {
        return 0;
    }

    let generated = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let sem = Arc::new(Semaphore::new(MEDIA_PARALLELISM));
    let mut handles = Vec::with_capacity(work.len());

    for (photo_id, source_path, thumb_abs, mime_type) in work {
        let sem = sem.clone();
        let gen = generated.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            if generate_thumbnail_file(&source_path, &thumb_abs, &mime_type, None).await {
                gen.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(photo_id = %photo_id, "Phase 1: generated thumbnail");
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    generated.load(Ordering::Relaxed)
}

// ── Phase 2: Convert flagged files to browser-friendly formats ──────────────

/// Convert every file that needs a web preview but doesn't have one yet.
/// Returns `(converted_count, ids_of_converted_files)`.
///
/// Original files are **never deleted**.  The converted copy is written to
/// `.web_previews/{id}.web.{ext}` alongside the original.  Both remain
/// available: `/photos/:id/file` serves the original and `/photos/:id/web`
/// serves the web-compatible preview.
///
/// For encrypted photos with the key available, the converted data is also
/// re-encrypted into a new blob so the client receives web-compatible media
/// after decryption.
async fn phase_convert(
    photos: &[PhotoRow],
    pool: &SqlitePool,
    storage_root: &PathBuf,
    encryption_key: &Arc<RwLock<Option<[u8; 32]>>>,
    key_available: bool,
) -> (u32, Vec<String>) {
    // ── Pass 1: Collect plain-disk conversions (parallelisable) ──────────
    // These only need FFmpeg and no DB writes (except optional re-encrypt),
    // so they are safe to run concurrently.
    struct DiskWork {
        photo_id: String,
        filename: String,
        source_path: PathBuf,
        preview_path: PathBuf,
        preview_ext: &'static str,
        is_encrypted: bool,
        encrypted_blob_id: Option<String>,
        user_id: String,
    }

    let mut disk_work: Vec<DiskWork> = Vec::new();
    let mut mark_converted_ids: Vec<String> = Vec::new();
    // Items that need decrypt→convert→re-encrypt (done sequentially to
    // limit DB write contention and memory usage from decrypted blobs).
    struct EncWork {
        photo_id: String,
        filename: String,
        encrypted_blob_id: String,
        user_id: String,
        preview_ext: &'static str,
    }
    let mut enc_work: Vec<EncWork> = Vec::new();

    for (photo_id, file_path, filename, _thumb_path, _mime_type, encrypted_blob_id, user_id) in photos {
        let is_encrypted = encrypted_blob_id.is_some();
        let preview_ext = match needs_web_preview(filename) {
            Some(ext) => ext,
            None => continue,
        };

        let source_path = storage_root.join(file_path);
        if tokio::fs::try_exists(&source_path).await.unwrap_or(false) {
            let preview_path = storage_root.join(format!(
                ".web_previews/{}.web.{}",
                photo_id, preview_ext
            ));
            if !tokio::fs::try_exists(&preview_path).await.unwrap_or(false) {
                // Skip files whose conversion has permanently failed
                if is_conversion_failed(pool, photo_id).await {
                    continue;
                }
                disk_work.push(DiskWork {
                    photo_id: photo_id.clone(),
                    filename: filename.clone(),
                    source_path,
                    preview_path,
                    preview_ext,
                    is_encrypted,
                    encrypted_blob_id: encrypted_blob_id.clone(),
                    user_id: user_id.clone(),
                });
            } else if is_encrypted {
                mark_converted_ids.push(photo_id.clone());
            }
            continue;
        }

        // Path B: encrypted-only (no source on disk)
        if is_encrypted && key_available {
            if let Some(blob_id) = encrypted_blob_id.as_deref() {
                if !is_blob_already_converted(pool, photo_id).await
                    && !is_conversion_failed(pool, photo_id).await
                {
                    enc_work.push(EncWork {
                        photo_id: photo_id.clone(),
                        filename: filename.clone(),
                        encrypted_blob_id: blob_id.to_string(),
                        user_id: user_id.clone(),
                        preview_ext,
                    });
                }
            }
        }
    }

    // Batch-mark already-converted encrypted photos
    for pid in &mark_converted_ids {
        mark_blob_converted(pool, pid).await;
        tracing::debug!(photo_id = %pid, "Phase 2: marked encrypted photo as converted (preview on disk)");
    }

    // ── Run disk conversions in parallel ─────────────────────────────────
    let converted = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let converted_ids = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let sem = Arc::new(Semaphore::new(MEDIA_PARALLELISM));

    let mut handles = Vec::with_capacity(disk_work.len());
    for item in disk_work {
        let sem = sem.clone();
        let conv_count = converted.clone();
        let conv_ids = converted_ids.clone();
        let pool = pool.clone();
        let storage_root = storage_root.clone();
        let enc_key = encryption_key.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            tracing::info!(
                photo_id = %item.photo_id,
                filename = %item.filename,
                target_ext = item.preview_ext,
                encrypted = item.is_encrypted,
                "Phase 2: starting conversion (source on disk)"
            );
            if generate_web_preview_bg(&item.source_path, &item.preview_path, item.preview_ext).await {
                conv_count.fetch_add(1, Ordering::Relaxed);
                conv_ids.lock().await.push(item.photo_id.clone());
                tracing::info!(
                    photo_id = %item.photo_id,
                    filename = %item.filename,
                    "Phase 2: conversion complete (source on disk)"
                );
                // Re-encrypt the blob if encrypted
                if item.is_encrypted && key_available {
                    if let Some(enc_id) = item.encrypted_blob_id.as_deref() {
                        if let Err(e) = reencrypt_blob_with_converted_data(
                            &pool, &storage_root, &enc_key,
                            &item.photo_id, &item.user_id,
                            enc_id, &item.filename, &item.preview_path, item.preview_ext,
                        ).await {
                            tracing::warn!(
                                photo_id = %item.photo_id,
                                "Phase 2: re-encryption failed: {}", e
                            );
                        }
                    }
                }
            } else {
                tracing::warn!(
                    photo_id = %item.photo_id,
                    filename = %item.filename,
                    "Phase 2: conversion failed (source on disk)"
                );
                // Mark as failed so we don't retry every 60-second cycle
                mark_conversion_failed(&pool, &item.photo_id).await;
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    // ── Run encrypted-blob conversions sequentially ──────────────────────
    // These involve decrypt→convert→re-encrypt with DB writes; sequential
    // avoids heavy memory use from concurrent decrypted blobs.
    for item in enc_work {
        tracing::info!(
            photo_id = %item.photo_id,
            filename = %item.filename,
            target_ext = item.preview_ext,
            "Phase 2: starting conversion (temp-decrypt encrypted blob)"
        );
        match decrypt_convert_reencrypt(
            pool, storage_root, encryption_key,
            &item.photo_id, &item.user_id,
            &item.encrypted_blob_id, &item.filename, item.preview_ext,
        ).await {
            Ok(true) => {
                converted.fetch_add(1, Ordering::Relaxed);
                converted_ids.lock().await.push(item.photo_id.clone());
                tracing::info!(
                    photo_id = %item.photo_id,
                    filename = %item.filename,
                    "Phase 2: conversion complete (re-encrypted blob)"
                );
            }
            Ok(false) => {
                tracing::warn!(
                    photo_id = %item.photo_id,
                    filename = %item.filename,
                    "Phase 2: conversion skipped/failed (encrypted blob)"
                );
                mark_conversion_failed(pool, &item.photo_id).await;
            }
            Err(e) => {
                tracing::error!(
                    photo_id = %item.photo_id,
                    filename = %item.filename,
                    "Phase 2: decrypt/convert/reencrypt error: {}", e
                );
                mark_conversion_failed(pool, &item.photo_id).await;
            }
        }
    }

    let final_count = converted.load(Ordering::Relaxed);
    let final_ids = converted_ids.lock().await.clone();
    (final_count, final_ids)
}

// ── Phase 3: Post-conversion thumbnails (freshly converted only) ────────────

/// Regenerate thumbnails for files that were just converted in Phase 2.
/// The web-previewed file (e.g. the .mp4 from a .mkv) may yield a better
/// thumbnail than the original exotic format.  Only files whose ID is in
/// `converted_ids` are processed — everything else was already handled in
/// Phase 1.
async fn phase_post_conversion_thumbnails(
    photos: &[PhotoRow],
    storage_root: &PathBuf,
    converted_ids: &[String],
) -> u32 {
    if converted_ids.is_empty() {
        return 0;
    }

    let id_set: std::collections::HashSet<&str> =
        converted_ids.iter().map(|s| s.as_str()).collect();

    // Collect work items first, then run in parallel.
    struct ThumbWork {
        photo_id: String,
        thumb_source: PathBuf,
        thumb_abs: PathBuf,
        preview_mime: String,
    }
    let mut work: Vec<ThumbWork> = Vec::new();

    for (photo_id, _file_path, filename, thumb_path, mime_type, encrypted_blob_id, _user_id) in photos {
        if !id_set.contains(photo_id.as_str()) {
            continue;
        }
        if encrypted_blob_id.is_some() {
            continue;
        }
        let tp = match thumb_path {
            Some(tp) => tp,
            None => continue,
        };
        let preview_ext = match needs_web_preview(filename) {
            Some(ext) => ext,
            None => continue,
        };
        let preview_path = storage_root.join(format!(
            ".web_previews/{}.web.{}",
            photo_id, preview_ext
        ));
        let thumb_abs = storage_root.join(tp);
        let thumb_source = if tokio::fs::try_exists(&preview_path).await.unwrap_or(false) {
            preview_path.clone()
        } else {
            continue; // No preview to generate thumbnail from
        };

        // Determine the preview mime type for thumbnail generation
        let preview_mime = match preview_ext {
            "jpg" => "image/jpeg",
            "png" => "image/png",
            "mp3" => "audio/mpeg",
            "mp4" => "video/mp4",
            _ => mime_type.as_str(),
        };
        work.push(ThumbWork {
            photo_id: photo_id.clone(),
            thumb_source,
            thumb_abs,
            preview_mime: preview_mime.to_string(),
        });
    }

    if work.is_empty() {
        return 0;
    }

    let generated = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let sem = Arc::new(Semaphore::new(MEDIA_PARALLELISM));
    let mut handles = Vec::with_capacity(work.len());

    for item in work {
        let sem = sem.clone();
        let gen = generated.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            if generate_thumbnail_file(&item.thumb_source, &item.thumb_abs, &item.preview_mime, None).await {
                gen.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(photo_id = %item.photo_id, "Phase 3: regenerated thumbnail from web preview");
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    generated.load(Ordering::Relaxed)
}

// ── Orchestrator: runs all three phases sequentially ────────────────────────

/// Run one complete processing cycle (three sequential phases).
///
/// Returns `(thumbnails_phase1, converted, thumbnails_phase3)`.
async fn run_conversion_pass(
    pool: &SqlitePool,
    read_pool: &SqlitePool,
    storage_root: &PathBuf,
    encryption_key: &Arc<RwLock<Option<[u8; 32]>>>,
) -> (u32, u32, u32) {
    // Skip entirely while encryption migration is running.  Converting during
    // encryption is confusing (two overlapping banners) and wasteful — the
    // migration-done handler will trigger conversion once encryption finishes.
    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(read_pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "idle".to_string());
    if mig_status == "encrypting" || mig_status == "decrypting" {
        tracing::info!("[DIAG:CONVERT] run_conversion_pass SKIPPED — migration in progress (status={})", mig_status);
        return (0, 0, 0);
    }

    // Diagnostic: log encrypted photos missing thumbnail blobs
    let enc_missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NOT NULL AND encrypted_thumb_blob_id IS NULL",
    )
    .fetch_one(read_pool)
    .await
    .unwrap_or(0);
    if enc_missing > 0 {
        tracing::warn!(
            "[DIAG:CONVERT] {} encrypted photo(s) missing encrypted_thumb_blob_id",
            enc_missing
        );
    }

    // Check FFmpeg availability (cached — only spawns subprocess once)
    let has_ffmpeg = ffmpeg_available_cached().await;
    if !has_ffmpeg {
        tracing::info!("[DIAG:CONVERT] run_conversion_pass SKIPPED — FFmpeg not available");
        return (0, 0, 0);
    }

    let key_available = encryption_key.read().await.is_some();

    // Check audio backup toggle — skip audio files when disabled
    let audio_backup_enabled: bool = sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(read_pool)
    .await
    .ok()
    .flatten()
    .map(|v| v == "true")
    .unwrap_or(true); // default: enabled

    // Fetch all photos once — shared across all three phases (read_pool)
    let all_photos: Vec<PhotoRow> = match sqlx::query_as(
        "SELECT id, file_path, filename, thumb_path, mime_type, encrypted_blob_id, user_id FROM photos",
    )
    .fetch_all(read_pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("[DIAG:CONVERT] DB query failed: {}", e);
            return (0, 0, 0);
        }
    };

    // Filter out audio files when the toggle is off
    let photos: Vec<PhotoRow> = if audio_backup_enabled {
        all_photos
    } else {
        all_photos.into_iter()
            .filter(|p| !p.4.starts_with("audio/"))
            .collect()
    };

    let plain_count = photos.iter().filter(|p| p.5.is_none()).count();
    let enc_count = photos.iter().filter(|p| p.5.is_some()).count();
    tracing::info!(
        "[DIAG:CONVERT] run_conversion_pass — {} photos ({} plain, {} encrypted, key_available={})",
        photos.len(), plain_count, enc_count, key_available
    );

    // ── Phase 1: Generate ALL missing thumbnails ────────────────────────
    let thumbs1 = phase_thumbnails(&photos, storage_root).await;
    if thumbs1 > 0 {
        tracing::info!("[DIAG:CONVERT] Phase 1 complete: generated {} thumbnails", thumbs1);
    }

    // ── Phase 2: Convert flagged files ──────────────────────────────────
    let (converted, converted_ids) =
        phase_convert(&photos, pool, storage_root, encryption_key, key_available).await;
    if converted > 0 {
        tracing::info!("[DIAG:CONVERT] Phase 2 complete: converted {} files", converted);
    }

    // ── Phase 3: Post-conversion thumbnails (freshly converted only) ────
    let thumbs3 = phase_post_conversion_thumbnails(&photos, storage_root, &converted_ids).await;
    if thumbs3 > 0 {
        tracing::info!("[DIAG:CONVERT] Phase 3 complete: regenerated {} thumbnails from converted files", thumbs3);
    }

    if thumbs1 > 0 || converted > 0 || thumbs3 > 0 {
        tracing::info!(
            "Processing pipeline complete: {} initial thumbnails, {} conversions, {} post-conversion thumbnails",
            thumbs1, converted, thumbs3
        );
    }

    (thumbs1, converted, thumbs3)
}

/// Check if an encrypted blob has already been converted.
///
/// Uses a lightweight marker in the `server_settings` DB table
/// (key = `blob_converted_{photo_id}`, value = `"true"`).
async fn is_blob_already_converted(pool: &SqlitePool, photo_id: &str) -> bool {
    // Query the DB marker set by `mark_blob_converted` after
    // successful re-encryption with web-compatible data.
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = ?",
    )
    .bind(format!("blob_converted_{}", photo_id))
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    marker.is_some()
}

/// Mark an encrypted blob as having been converted.
async fn mark_blob_converted(pool: &SqlitePool, photo_id: &str) {
    if let Err(e) = sqlx::query(
        "INSERT OR REPLACE INTO server_settings (key, value) VALUES (?, 'true')",
    )
    .bind(format!("blob_converted_{}", photo_id))
    .execute(pool)
    .await
    {
        // Without this marker the photo will be redundantly re-converted on
        // every background cycle — wasteful but not data-corrupting.
        tracing::warn!(photo_id = photo_id, error = %e, "Failed to mark blob as converted");
    }
}

/// Mark a photo's conversion as permanently failed.
///
/// Once marked, the background pipeline skips this photo on subsequent cycles.
/// The marker uses `server_settings` with key `conv_failed_{photo_id}`.
async fn mark_conversion_failed(pool: &SqlitePool, photo_id: &str) {
    let _ = sqlx::query(
        "INSERT OR REPLACE INTO server_settings (key, value) VALUES (?, 'true')",
    )
    .bind(format!("conv_failed_{}", photo_id))
    .execute(pool)
    .await;
}

/// Check whether a photo's conversion has been marked as permanently failed.
async fn is_conversion_failed(pool: &SqlitePool, photo_id: &str) -> bool {
    sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_settings WHERE key = ?",
    )
    .bind(format!("conv_failed_{}", photo_id))
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .is_some()
}

/// Decrypt an encrypted blob, extract the media payload, convert it to a
/// web-compatible format, re-encrypt with the converted data, and update
/// the blob in storage and DB.
async fn decrypt_convert_reencrypt(
    pool: &SqlitePool,
    storage_root: &PathBuf,
    encryption_key: &Arc<RwLock<Option<[u8; 32]>>>,
    photo_id: &str,
    user_id: &str,
    blob_id: &str,
    filename: &str,
    preview_ext: &str,
) -> Result<bool, String> {
    // Step 1: Read the encrypted blob from disk
    let blob_storage_path: String = sqlx::query_scalar(
        "SELECT storage_path FROM blobs WHERE id = ?",
    )
    .bind(blob_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB query for blob path: {}", e))?
    .ok_or_else(|| format!("Blob {} not found in DB", blob_id))?;

    let enc_data = storage::read_blob(storage_root, &blob_storage_path)
        .await
        .map_err(|e| format!("Read blob failed: {}", e))?;

    // Step 2: Decrypt
    let key = encryption_key.read().await
        .ok_or_else(|| "Encryption key no longer available".to_string())?;
    let plaintext = {
        let key_copy = key;
        tokio::task::spawn_blocking(move || crypto::decrypt(&key_copy, &enc_data))
            .await
            .map_err(|e| format!("Decrypt task panicked: {}", e))?
            .map_err(|e| format!("Decryption failed: {}", e))?
    };

    // Step 3: Parse the JSON payload to extract the media data
    let payload: serde_json::Value = serde_json::from_slice(&plaintext)
        .map_err(|e| format!("JSON parse failed: {}", e))?;

    let payload_mime = payload.get("mime_type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Check if the payload is already in a web-compatible format
    let target_mime = match preview_ext {
        "jpg" => "image/jpeg",
        "png" => "image/png",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        _ => return Ok(false),
    };
    if payload_mime == target_mime {
        tracing::info!(
            photo_id = %photo_id,
            "Blob already contains converted data (mime={}), marking as converted",
            payload_mime
        );
        mark_blob_converted(pool, photo_id).await;
        return Ok(false);
    }

    // Step 4: Extract the base64 media data
    let media_b64 = payload.get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No 'data' field in payload".to_string())?;
    let media_data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, media_b64)
        .map_err(|e| format!("Base64 decode failed: {}", e))?;

    // Step 5: Write to temp file, convert, read result
    let original_ext = filename.rsplit('.').next().unwrap_or("bin");
    let tmp_input = std::env::temp_dir().join(format!(
        "sp_conv_in_{}.{}",
        Uuid::new_v4(), original_ext
    ));
    let tmp_output = std::env::temp_dir().join(format!(
        "sp_conv_out_{}.{}",
        Uuid::new_v4(), preview_ext
    ));

    tokio::fs::write(&tmp_input, &media_data)
        .await
        .map_err(|e| format!("Write temp input: {}", e))?;

    let conversion_ok = generate_web_preview_bg(&tmp_input, &tmp_output, preview_ext).await;
    let _ = tokio::fs::remove_file(&tmp_input).await;

    if !conversion_ok {
        let _ = tokio::fs::remove_file(&tmp_output).await;
        return Ok(false);
    }

    let converted_data = tokio::fs::read(&tmp_output)
        .await
        .map_err(|e| format!("Read converted output: {}", e))?;
    let _ = tokio::fs::remove_file(&tmp_output).await;

    if converted_data.is_empty() {
        return Ok(false);
    }

    // Step 6: Re-encrypt with the converted data
    reencrypt_payload(
        pool, storage_root, encryption_key, photo_id, user_id,
        blob_id, &payload, &converted_data, target_mime,
    ).await?;

    mark_blob_converted(pool, photo_id).await;
    Ok(true)
}

/// Re-encrypt the blob with already-converted data from a web preview file.
async fn reencrypt_blob_with_converted_data(
    pool: &SqlitePool,
    storage_root: &PathBuf,
    encryption_key: &Arc<RwLock<Option<[u8; 32]>>>,
    photo_id: &str,
    user_id: &str,
    blob_id: &str,
    _filename: &str,
    preview_path: &std::path::Path,
    preview_ext: &str,
) -> Result<(), String> {
    // Read the encrypted blob and decrypt
    let blob_storage_path: String = sqlx::query_scalar(
        "SELECT storage_path FROM blobs WHERE id = ?",
    )
    .bind(blob_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB query for blob path: {}", e))?
    .ok_or_else(|| format!("Blob {} not found in DB", blob_id))?;

    let enc_data = storage::read_blob(storage_root, &blob_storage_path)
        .await
        .map_err(|e| format!("Read blob failed: {}", e))?;

    let key = encryption_key.read().await
        .ok_or_else(|| "Encryption key no longer available".to_string())?;
    let plaintext = {
        let key_copy = key;
        tokio::task::spawn_blocking(move || crypto::decrypt(&key_copy, &enc_data))
            .await
            .map_err(|e| format!("Decrypt panicked: {}", e))?
            .map_err(|e| format!("Decryption failed: {}", e))?
    };

    let payload: serde_json::Value = serde_json::from_slice(&plaintext)
        .map_err(|e| format!("JSON parse: {}", e))?;

    let target_mime = match preview_ext {
        "jpg" => "image/jpeg",
        "png" => "image/png",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        _ => return Err("Unknown preview ext".to_string()),
    };

    // Check if already converted
    let payload_mime = payload.get("mime_type").and_then(|v| v.as_str()).unwrap_or("");
    if payload_mime == target_mime {
        mark_blob_converted(pool, photo_id).await;
        return Ok(());
    }

    let converted_data = tokio::fs::read(preview_path)
        .await
        .map_err(|e| format!("Read preview file: {}", e))?;

    reencrypt_payload(
        pool, storage_root, encryption_key, photo_id, user_id,
        blob_id, &payload, &converted_data, target_mime,
    ).await?;

    mark_blob_converted(pool, photo_id).await;
    Ok(())
}

/// Build a new encrypted blob from the original payload JSON but with
/// converted media data, write it to storage, and update the DB.
async fn reencrypt_payload(
    pool: &SqlitePool,
    storage_root: &PathBuf,
    encryption_key: &Arc<RwLock<Option<[u8; 32]>>>,
    photo_id: &str,
    user_id: &str,
    old_blob_id: &str,
    original_payload: &serde_json::Value,
    converted_data: &[u8],
    new_mime: &str,
) -> Result<(), String> {
    // Build updated payload with converted data and new mime type
    let mut new_payload = original_payload.clone();
    if let Some(obj) = new_payload.as_object_mut() {
        obj.insert("data".to_string(), serde_json::Value::String(
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, converted_data)
        ));
        obj.insert("mime_type".to_string(), serde_json::Value::String(new_mime.to_string()));
    }

    let payload_json = serde_json::to_vec(&new_payload)
        .map_err(|e| format!("JSON serialize: {}", e))?;

    // Encrypt with the key
    let key = encryption_key.read().await
        .ok_or_else(|| "Encryption key no longer available".to_string())?;
    let enc_data = {
        let key_copy = key;
        let json_clone = payload_json;
        tokio::task::spawn_blocking(move || crypto::encrypt(&key_copy, &json_clone))
            .await
            .map_err(|e| format!("Encrypt panicked: {}", e))?
            .map_err(|e| format!("Encryption failed: {}", e))?
    };

    let enc_hash = hex::encode(Sha256::digest(&enc_data));
    let new_blob_id = Uuid::new_v4().to_string();

    // Determine blob_type from original blob
    let blob_type: String = sqlx::query_scalar(
        "SELECT blob_type FROM blobs WHERE id = ?",
    )
    .bind(old_blob_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB query blob_type: {}", e))?
    .unwrap_or_else(|| "photo".to_string());

    let blob_storage_path =
        storage::write_blob(storage_root, user_id, &new_blob_id, &enc_data)
            .await
            .map_err(|e| format!("Write blob: {}", e))?;

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&new_blob_id)
    .bind(user_id)
    .bind(&blob_type)
    .bind(enc_data.len() as i64)
    .bind(&enc_hash)
    .bind(&now)
    .bind(&blob_storage_path)
    .execute(pool)
    .await
    .map_err(|e| format!("Insert blob row: {}", e))?;

    // Update photo to point to the new blob
    sqlx::query("UPDATE photos SET encrypted_blob_id = ? WHERE id = ?")
        .bind(&new_blob_id)
        .bind(photo_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Update photo blob ref: {}", e))?;

    tracing::info!(
        photo_id = %photo_id,
        old_blob_id = %old_blob_id,
        new_blob_id = %new_blob_id,
        new_mime = %new_mime,
        "Re-encrypted blob with converted data"
    );

    // Optionally clean up old blob (mark for deletion but don't delete
    // immediately in case there are active downloads)
    // We'll leave the old blob for now — it will be orphaned and can be
    // cleaned up by a future garbage collection pass.

    Ok(())
}

/// Run the background processing pipeline loop.
///
/// Runs the three-phase pipeline (thumbnails → conversion → post-conversion
/// thumbnails) every `interval_secs` seconds, or immediately when notified
/// via the `notify` handle (e.g. after scan/migration completes).
///
/// The `active` flag is set while the pipeline is working, allowing the
/// conversion-status endpoint to show a progress banner in the UI.
///
/// The `encryption_key` handle is checked each cycle — when available, the
/// pipeline can process encrypted blobs by decrypting, converting, and
/// re-encrypting.
pub async fn background_convert_task(
    pool: SqlitePool,
    read_pool: SqlitePool,
    storage_root: PathBuf,
    interval_secs: u64,
    notify: Arc<Notify>,
    active: Arc<AtomicBool>,
    encryption_key: Arc<RwLock<Option<[u8; 32]>>>,
) {
    // Brief startup delay to let the server initialize before background work
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    loop {
        // Wait for either the timer or an explicit trigger
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(interval_secs)) => {
                tracing::info!("[DIAG:CONVERT] loop woke — 60s timer");
            },
            _ = notify.notified() => {
                tracing::info!("[DIAG:CONVERT] loop woke — notify trigger, coalescing 500ms");
                // Small delay so multiple rapid triggers coalesce
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            },
        }

        // Mark active before the pipeline so the banner shows immediately
        active.store(true, Ordering::Release);

        // All three phases run sequentially inside run_conversion_pass.
        // Phase 3 (post-conversion thumbnails) is handled internally — no
        // separate "second pass" needed.
        let (thumbs1, converted, thumbs3) =
            run_conversion_pass(&pool, &read_pool, &storage_root, &encryption_key).await;
        tracing::info!(
            "[DIAG:CONVERT] pipeline result: thumbs_p1={}, converted={}, thumbs_p3={}, setting active={}",
            thumbs1, converted, thumbs3, converted > 0 || thumbs1 > 0 || thumbs3 > 0
        );

        // Always clear the active flag after the pipeline completes.
        active.store(false, Ordering::Release);
        tracing::info!("[DIAG:CONVERT] active flag cleared to false");
    }
}

/// Admin endpoint: trigger an immediate processing pipeline cycle.
///
/// Wakes the background task to run all three phases (thumbnails → conversion
/// → post-conversion thumbnails) without waiting for the next timer tick.
///
/// POST /admin/photos/convert
pub async fn trigger_convert(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    require_admin(&state, &auth).await?;

    state.convert_notify.notify_one();

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "message": "Conversion triggered" })),
    ))
}

/// Admin endpoint: supply the encryption key and trigger re-conversion of
/// encrypted blobs that still contain raw (non-web-compatible) media data.
///
/// This is used when encryption migration already completed but the blobs
/// were not converted at the time (e.g. the pipeline was skipping during
/// migration, or no FFmpeg was available).  The key is stored temporarily
/// in `AppState` for 30 minutes, during which the background pipeline will
/// decrypt, convert, and re-encrypt each blob.
///
/// POST /admin/photos/reconvert
///
/// Request body: `{ "key_hex": "<64-char hex string>" }`
pub async fn trigger_reconvert(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    require_admin(&state, &auth).await?;

    // Parse the encryption key from the request body
    let key_hex = req.get("key_hex")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'key_hex' field".into()))?;

    let key = crate::crypto::parse_key_hex(key_hex)
        .map_err(|e| AppError::BadRequest(format!("Invalid key: {}", e)))?;

    // Validate the key by attempting to decrypt one encrypted blob
    let test_blob: Option<(String,)> = sqlx::query_as(
        "SELECT b.storage_path FROM photos p \
         JOIN blobs b ON b.id = p.encrypted_blob_id \
         WHERE p.encrypted_blob_id IS NOT NULL LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?;

    if let Some((storage_path,)) = test_blob {
        let storage_root = (**state.storage_root.load()).clone();
        let test_data = storage::read_blob(&storage_root, &storage_path)
            .await
            .map_err(|e| AppError::Internal(format!("Read test blob: {}", e)))?;

        let key_copy = key;
        let decrypt_result = tokio::task::spawn_blocking(move || crypto::decrypt(&key_copy, &test_data))
            .await
            .map_err(|e| AppError::Internal(format!("Decrypt task panicked: {}", e)))?;

        if decrypt_result.is_err() {
            return Err(AppError::BadRequest(
                "Key validation failed: could not decrypt a test blob. Wrong key?".into(),
            ));
        }
    }

    // Count how many encrypted photos need conversion.
    // LIMIT 10000 to prevent OOM on massive libraries — this is a diagnostic
    // count, not an exhaustive scan.
    let photos: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, filename FROM photos WHERE encrypted_blob_id IS NOT NULL LIMIT 10000",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut needs_conversion = 0u32;
    for (id, filename) in &photos {
        if super::scan::needs_web_preview(filename).is_some() {
            if !is_blob_already_converted(&state.pool, id).await {
                needs_conversion += 1;
            }
        }
    }

    // Store the key in AppState
    {
        let mut guard = state.encryption_key.write().await;
        *guard = Some(key);
    }
    tracing::info!(
        "[RECONVERT] Key stored. {} encrypted photos need conversion.",
        needs_conversion
    );

    // Trigger the converter immediately
    state.convert_notify.notify_one();

    // Spawn a timer to clear the key after 30 minutes
    let key_store = state.encryption_key.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30 * 60)).await;
        let mut guard = key_store.write().await;
        if guard.is_some() {
            *guard = None;
            tracing::info!("[RECONVERT] Encryption key cleared after 30-minute grace period");
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "message": format!("Re-conversion triggered for {} encrypted photos.", needs_conversion),
            "needs_conversion": needs_conversion,
        })),
    ))
}

/// Check how many files still need processing for the current user.
///
/// GET /photos/conversion-status
///
/// Reports the number of photos pending in each pipeline phase:
/// - `missing_thumbnails`: Phase 1 work remaining
/// - `pending_conversions`: Phase 2 work remaining (browser-incompatible formats)
/// - `pending_awaiting_key`: Encrypted files that need the key before conversion
/// - `converting`: Whether the pipeline is currently active
///
/// Original files are always preserved after conversion.  The converted copy
/// lives in `.web_previews/` and the original is served via `/photos/:id/file`.
pub async fn conversion_status(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let storage_root = (**state.storage_root.load()).clone();
    let converting = state.conversion_active.load(Ordering::Acquire);
    let key_available = state.encryption_key.read().await.is_some();

    // ── Check encryption mode (read_pool — these are read-only) ─────────
    let enc_mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.read_pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "plain".to_string());

    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(&state.read_pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "idle".to_string());

    let migration_running = mig_status == "encrypting" || mig_status == "decrypting";

    // ── Count missing thumbnails via SQL (plain photos with thumb_path) ──
    // This avoids loading all rows and checking the filesystem for each.
    // We still need filesystem checks for conversion status, but we can
    // use a smarter SQL filter to limit the set.
    let _missing_thumbnails_base: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos \
         WHERE user_id = ? AND encrypted_blob_id IS NULL AND thumb_path IS NOT NULL",
    )
    .bind(&_auth.user_id)
    .fetch_one(&state.read_pool)
    .await
    .unwrap_or(0);

    // For the missing thumbnail count, we still need filesystem checks since
    // the DB doesn't track whether the file exists. But we can do this more
    // efficiently by only checking the relevant subset.
    let thumb_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, thumb_path FROM photos \
         WHERE user_id = ? AND encrypted_blob_id IS NULL AND thumb_path IS NOT NULL \
         LIMIT 10000",
    )
    .bind(&_auth.user_id)
    .fetch_all(&state.read_pool)
    .await?;

    let mut missing_thumbnails = 0u32;
    // Batch filesystem checks using concurrent tasks
    let mut thumb_checks = Vec::with_capacity(thumb_rows.len());
    for (_id, tp) in &thumb_rows {
        let path = storage_root.join(tp);
        thumb_checks.push(tokio::fs::try_exists(path));
    }
    let thumb_results = futures_util::future::join_all(thumb_checks).await;
    for exists in thumb_results {
        if !exists.unwrap_or(false) {
            missing_thumbnails += 1;
        }
    }

    // ── Check photos needing web-preview conversion ─────────────────────
    // Only fetch photos whose filename extension matches needs_web_preview.
    // SQL LIKE can't perfectly match the Rust logic, so we fetch a broader
    // set and filter in Rust, but only for convertible extensions.
    let convertible_photos: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, filename, file_path, encrypted_blob_id FROM photos \
         WHERE user_id = ? LIMIT 10000",
    )
    .bind(&_auth.user_id)
    .fetch_all(&state.read_pool)
    .await?;

    let mut pending_conversions = 0u32;
    let mut pending_awaiting_key = 0u32;

    // Collect filesystem-check futures for plain photos
    struct ConvCheck {
        is_encrypted: bool,
        preview_path: PathBuf,
        source_path: Option<PathBuf>,
        photo_id: String,
    }
    let mut checks: Vec<ConvCheck> = Vec::new();

    for (id, filename, file_path, encrypted_blob_id) in &convertible_photos {
        let ext = match super::scan::needs_web_preview(filename) {
            Some(e) => e,
            None => continue,
        };
        let is_encrypted = encrypted_blob_id.is_some();
        let preview_path = storage_root.join(format!(".web_previews/{}.web.{}", id, ext));

        if is_encrypted {
            checks.push(ConvCheck {
                is_encrypted: true,
                preview_path,
                source_path: None,
                photo_id: id.clone(),
            });
        } else {
            checks.push(ConvCheck {
                is_encrypted: false,
                preview_path,
                source_path: Some(storage_root.join(file_path)),
                photo_id: id.clone(),
            });
        }
    }

    // Run all filesystem existence checks concurrently
    let mut preview_futs = Vec::with_capacity(checks.len());
    let mut source_futs = Vec::with_capacity(checks.len());
    for c in &checks {
        preview_futs.push(tokio::fs::try_exists(&c.preview_path));
        if let Some(ref sp) = c.source_path {
            source_futs.push(Some(tokio::fs::try_exists(sp)));
        } else {
            source_futs.push(None);
        }
    }

    let preview_results = futures_util::future::join_all(preview_futs).await;
    // For source checks, only join the Some() ones
    let mut source_results: Vec<Option<bool>> = Vec::with_capacity(checks.len());
    for sf in source_futs {
        match sf {
            Some(fut) => source_results.push(Some(fut.await.unwrap_or(false))),
            None => source_results.push(None),
        }
    }

    for (i, c) in checks.iter().enumerate() {
        let preview_exists = preview_results[i].as_ref().copied().unwrap_or(false);
        if preview_exists {
            continue; // Already converted
        }

        // Skip items whose conversion permanently failed
        if is_conversion_failed(&state.read_pool, &c.photo_id).await {
            continue;
        }

        if c.is_encrypted {
            let already_converted = is_blob_already_converted(&state.read_pool, &c.photo_id).await;
            if already_converted {
                continue;
            }
            if key_available && !migration_running {
                pending_conversions += 1;
            } else {
                pending_awaiting_key += 1;
            }
        } else {
            let source_exists = source_results[i].unwrap_or(false);
            if source_exists {
                pending_conversions += 1;
            }
        }
    }

    // ── Encrypted-mode: count photos missing encrypted thumbnail blobs ──
    let enc_missing_thumbs: i64 = if enc_mode == "encrypted" {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NOT NULL AND encrypted_thumb_blob_id IS NULL AND user_id = ?",
        )
        .bind(&_auth.user_id)
        .fetch_one(&state.read_pool)
        .await
        .unwrap_or(0)
    } else {
        0
    };

    if !migration_running && enc_missing_thumbs > 0 {
        missing_thumbnails += enc_missing_thumbs as u32;
    }

    if pending_conversions > 0 || pending_awaiting_key > 0 || missing_thumbnails > 0 || converting || enc_missing_thumbs > 0 {
        tracing::info!(
            "[DIAG:STATUS] conversion-status: pending={}, awaiting_key={}, thumbs={}, converting={}, total_photos={}, enc_mode={}, mig_status={}, enc_missing_thumbs={}, key_available={}",
            pending_conversions, pending_awaiting_key, missing_thumbnails, converting, convertible_photos.len(), enc_mode, mig_status, enc_missing_thumbs, key_available
        );
    }

    Ok(Json(serde_json::json!({
        "pending_conversions": pending_conversions,
        "pending_awaiting_key": pending_awaiting_key,
        "missing_thumbnails": missing_thumbnails,
        "converting": converting,
        "enc_missing_thumbs": enc_missing_thumbs,
        "key_available": key_available,
        "migration_running": migration_running,
    })))
}
