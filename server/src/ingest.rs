//! Conversion ingest engine — runs AFTER native file import and encryption
//! are complete. Discovers non-native media files on disk, converts them to
//! browser-compatible formats via FFmpeg, registers the converted files in
//! the database, and triggers encryption for the newly registered files.
//!
//! This module enforces strict sequencing:
//!   1. Native files are imported and encrypted (handled by scan + server_migrate)
//!   2. THIS module then discovers convertible files
//!   3. Converts them to a `.converted/` staging folder
//!   4. Registers the converted results in the DB
//!   5. Triggers encryption for the newly converted files
//!
//! This prevents the race condition where conversion and encryption run
//! simultaneously on different files.

use std::path::PathBuf;

use futures_util::TryStreamExt;
use uuid::Uuid;

use crate::conversion;
use crate::photos::metadata::extract_media_metadata_async;
use crate::photos::thumbnail::generate_thumbnail_file;
use crate::photos::utils::{compute_photo_hash_streaming, normalize_iso_timestamp, utc_now_iso};

/// Run a conversion pass: discover non-native files, convert, register, encrypt.
///
/// Called AFTER `auto_migrate_after_scan()` completes so that native files
/// are fully encrypted before we start FFmpeg conversions.
pub async fn run_conversion_pass(
    pool: sqlx::SqlitePool,
    storage_root: PathBuf,
    jwt_secret: String,
) {
    // ── Step 0: Determine the admin user to assign new photos to ─────────
    let admin_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();

    let admin_id = match admin_id {
        Some(id) => id,
        None => {
            tracing::debug!("[INGEST] No admin user yet, skipping conversion pass");
            return;
        }
    };

    // ── Wait for Phase 1 encryption to finish ────────────────────────────
    // The ingest engine must not start until ALL native files are encrypted.
    // Poll the unencrypted count with a timeout so we don't spin forever if
    // no encryption key is stored yet.
    {
        let max_wait = std::time::Duration::from_secs(300); // 5 min ceiling
        let start = std::time::Instant::now();
        loop {
            let unencrypted: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NULL",
            )
            .fetch_one(&pool)
            .await
            .unwrap_or(0);

            if unencrypted == 0 {
                break;
            }

            // If no wrapped key exists, encryption can't proceed — don't block.
            let has_key: bool = sqlx::query_scalar(
                "SELECT COUNT(*) > 0 FROM server_settings WHERE key = 'encryption_key_wrapped'",
            )
            .fetch_one(&pool)
            .await
            .unwrap_or(false);

            if !has_key {
                tracing::info!(
                    "[INGEST] No encryption key stored, proceeding with conversion \
                     ({} photos still unencrypted)",
                    unencrypted
                );
                break;
            }

            if start.elapsed() > max_wait {
                tracing::warn!(
                    "[INGEST] Timed out waiting for encryption ({} still pending), proceeding",
                    unencrypted
                );
                break;
            }

            tracing::debug!(
                "[INGEST] Waiting for {} native photos to be encrypted before starting conversion",
                unencrypted
            );
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    // Check audio-backup toggle.
    let audio_enabled: bool = sqlx::query_scalar(
        "SELECT value = 'true' FROM server_settings WHERE key = 'audio_backup_enabled'",
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(false);

    // ── Step 1: Build set of already-known paths ─────────────────────────
    let mut existing_set = std::collections::HashSet::new();
    {
        let mut rows = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM photos WHERE file_path != '' \
             UNION SELECT source_path FROM photos WHERE source_path IS NOT NULL AND source_path != '' \
             UNION SELECT file_path FROM trash_items WHERE file_path != ''",
        )
        .fetch(&pool);

        while let Some(path) = rows.try_next().await.unwrap_or(None) {
            existing_set.insert(path);
        }
    }

    // ── Step 2: Walk directory and collect convertible candidates ─────────
    struct ConvertCandidate {
        abs_path: PathBuf,
        rel_path: String,
        name: String,
        target: conversion::ConversionTarget,
        size: i64,
        modified: Option<String>,
    }

    let mut candidates: Vec<ConvertCandidate> = Vec::new();
    let mut queue = vec![storage_root.clone()];

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
                } else if ft.is_file() {
                    // Only look at files that need conversion (NOT native media).
                    let target = match conversion::conversion_target(&name) {
                        Some(t) => t,
                        None => continue,
                    };

                    let abs_path = entry.path();
                    let rel_path = abs_path
                        .strip_prefix(&storage_root)
                        .unwrap_or(&abs_path)
                        .to_string_lossy()
                        .replace('\\', "/");

                    // Skip if already processed (source_path in DB).
                    if existing_set.contains(&rel_path) {
                        continue;
                    }

                    // Skip audio when toggle is off.
                    if target.category == conversion::MediaCategory::Audio && !audio_enabled {
                        continue;
                    }

                    let file_meta = entry.metadata().await.ok();
                    let size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
                    let modified = file_meta.and_then(|m| {
                        m.modified().ok().map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            normalize_iso_timestamp(&dt.to_rfc3339())
                        })
                    });

                    candidates.push(ConvertCandidate {
                        abs_path,
                        rel_path,
                        name,
                        target,
                        size,
                        modified,
                    });
                }
            }
        }
    }

    if candidates.is_empty() {
        tracing::debug!("[INGEST] No convertible files found");
        return;
    }

    tracing::info!(
        "[INGEST] Found {} convertible files, starting conversion",
        candidates.len()
    );

    // ── Step 3: Convert all files to .converted/ staging folder ──────────
    conversion::progress_start(candidates.len() as i64);

    let conv_dir = storage_root.join(".converted");
    let mut registered = 0i64;

    for candidate in &candidates {
        let conv_id = Uuid::new_v4();
        let conv_filename = format!("{}.{}", conv_id, candidate.target.extension);
        let conv_abs = conv_dir.join(&conv_filename);
        let conv_rel = format!(".converted/{}", conv_filename);

        match conversion::convert_file(&candidate.abs_path, &conv_abs, &candidate.target).await {
            Ok(()) => {
                conversion::progress_tick();
                let new_name = {
                    let stem = std::path::Path::new(&candidate.name)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("converted");
                    format!("{}.{}", stem, candidate.target.extension)
                };
                tracing::info!(
                    original = %candidate.name,
                    converted = %new_name,
                    "[INGEST] Converted file to browser-native format"
                );

                // ── Step 4: Register the converted file in the DB ────
                let photo_id = Uuid::new_v4().to_string();
                let now = utc_now_iso();
                let work_mime = candidate.target.mime_type;
                let work_media_type = conversion::media_type_str(candidate.target.category);
                let thumb_ext = if work_mime == "image/gif" { "gif" } else { "jpg" };
                let thumb_rel = format!(".thumbnails/{}.thumb.{}", photo_id, thumb_ext);

                let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
                    extract_media_metadata_async(conv_abs.clone()).await;

                let final_taken_at = exif_taken
                    .map(|t| normalize_iso_timestamp(&t))
                    .or(candidate.modified.clone());

                let photo_hash = compute_photo_hash_streaming(&conv_abs).await;

                // Hash-based dedup: skip if an identical file was already registered
                // (catches re-conversion of the same source across concurrent scans).
                if let Some(ref hash) = photo_hash {
                    let dup_exists: bool = sqlx::query_scalar(
                        "SELECT COUNT(*) > 0 FROM photos WHERE photo_hash = ? AND user_id = ?",
                    )
                    .bind(hash)
                    .bind(&admin_id)
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(false);

                    if dup_exists {
                        tracing::debug!(
                            hash = %hash,
                            file = %candidate.name,
                            "[INGEST] Duplicate hash detected, skipping"
                        );
                        // Clean up the converted file we just created
                        let _ = tokio::fs::remove_file(&conv_abs).await;
                        continue;
                    }
                }

                let final_size = tokio::fs::metadata(&conv_abs)
                    .await
                    .map(|m| m.len() as i64)
                    .unwrap_or(candidate.size);

                let source_path = Some(candidate.rel_path.clone());

                let insert_result = sqlx::query(
                    "INSERT OR IGNORE INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                     size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, \
                     created_at, photo_hash, source_path) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&photo_id)
                .bind(&admin_id)
                .bind(&new_name)
                .bind(&conv_rel)
                .bind(work_mime)
                .bind(work_media_type)
                .bind(final_size)
                .bind(img_w)
                .bind(img_h)
                .bind(&final_taken_at)
                .bind(exif_lat)
                .bind(exif_lon)
                .bind(&cam_model)
                .bind(&thumb_rel)
                .bind(&now)
                .bind(&photo_hash)
                .bind(&source_path)
                .execute(&pool)
                .await;

                match insert_result {
                    Ok(result) if result.rows_affected() == 0 => {
                        tracing::debug!(
                            file = %conv_rel,
                            "[INGEST] Already registered (concurrent scan), skipping"
                        );
                        continue;
                    }
                    Err(e) => {
                        tracing::error!(
                            file = %conv_rel,
                            error = %e,
                            "[INGEST] Failed to register converted photo"
                        );
                        continue;
                    }
                    Ok(_) => {}
                }

                // Generate thumbnail for the converted file.
                let thumb_abs = storage_root.join(&thumb_rel);
                if generate_thumbnail_file(&conv_abs, &thumb_abs, work_mime, None).await {
                    tracing::debug!(file = %conv_rel, "[INGEST] Generated thumbnail");
                } else {
                    tracing::warn!(file = %conv_rel, "[INGEST] Failed to generate thumbnail");
                }

                registered += 1;
            }
            Err(e) => {
                conversion::progress_tick();
                tracing::warn!(
                    file = %candidate.name,
                    error = %e,
                    "[INGEST] Conversion failed, skipping file"
                );
            }
        }
    }

    conversion::progress_finish();

    tracing::info!(
        "[INGEST] Conversion pass complete: {}/{} files converted and registered",
        registered,
        candidates.len()
    );

    // ── Step 5: Encrypt the newly converted files ────────────────────────
    if registered > 0 {
        tracing::info!("[INGEST] Triggering encryption for {} newly converted files", registered);
        crate::photos::server_migrate::auto_migrate_after_scan(
            pool,
            storage_root,
            jwt_secret,
        )
        .await;
        tracing::info!("[INGEST] Encryption of converted files complete");
    }
}
