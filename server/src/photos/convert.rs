//! Background media conversion task.
//!
//! Periodically scans for photos/videos/audio that need web-compatible previews
//! but don't have them yet, and converts them in the background using low CPU
//! priority (`nice -n 19`).
//!
//! Video transcoding (MKV/AVI/WMV → MP4) is intentionally deferred to this
//! background task rather than running during scan, since it can take minutes
//! per file.
//!
//! The task can also be woken immediately via a `Notify` handle (e.g. after a
//! scan or upload completes).
//!
//! **Encrypted photo support**: When the encryption key is available in
//! `AppState` (stored temporarily during and after migration), the converter
//! can decrypt encrypted blobs, convert the media, and re-encrypt with the
//! web-compatible data. This makes conversion independent of encryption.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tokio::sync::{Notify, RwLock};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::blobs::storage;
use crate::crypto;
use crate::error::AppError;
use crate::state::AppState;

use super::scan::{needs_web_preview, generate_web_preview_bg, generate_thumbnail_file};

/// Run a single conversion + thumbnail pass. Returns (converted, thumbnails_generated).
///
/// When the encryption key is available, the converter can also process
/// encrypted photos by decrypting the blob, converting the media, and
/// re-encrypting with the web-compatible data.
async fn run_conversion_pass(
    pool: &SqlitePool,
    storage_root: &PathBuf,
    encryption_key: &Arc<RwLock<Option<[u8; 32]>>>,
) -> (u32, u32) {
    // Skip conversion entirely while encryption migration is running.
    // Converting during encryption is confusing (two overlapping banners)
    // and wasteful — the converter will be triggered once migration finishes.
    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "idle".to_string());
    if mig_status == "encrypting" || mig_status == "decrypting" {
        tracing::info!("[DIAG:CONVERT] run_conversion_pass SKIPPED — migration in progress (status={})", mig_status);
        return (0, 0);
    }

    // Log encrypted photos missing thumbnail blobs (diagnostic).
    let enc_missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NOT NULL AND encrypted_thumb_blob_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    if enc_missing > 0 {
        tracing::warn!(
            "[DIAG:CONVERT] {} encrypted photo(s) missing encrypted_thumb_blob_id",
            enc_missing
        );
    }

    // Check if FFmpeg is available
    let has_ffmpeg = super::scan::ffmpeg_available_pub().await;
    if !has_ffmpeg {
        tracing::info!("[DIAG:CONVERT] run_conversion_pass SKIPPED — FFmpeg not available");
        return (0, 0);
    }

    // Check if we have the encryption key available for encrypted blob processing
    let key_available = encryption_key.read().await.is_some();

    // Fetch ALL photos.
    let photos: Vec<(String, String, String, Option<String>, String, Option<String>, String)> = match sqlx::query_as(
        "SELECT id, file_path, filename, thumb_path, mime_type, encrypted_blob_id, user_id FROM photos",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("[DIAG:CONVERT] DB query failed: {}", e);
            return (0, 0);
        }
    };

    let plain_count = photos.iter().filter(|p| p.5.is_none()).count();
    let enc_count = photos.iter().filter(|p| p.5.is_some()).count();
    tracing::info!(
        "[DIAG:CONVERT] run_conversion_pass — {} photos ({} plain, {} encrypted, key_available={})",
        photos.len(), plain_count, enc_count, key_available
    );

    let mut converted = 0u32;
    let mut thumbnails_generated = 0u32;
    for (photo_id, file_path, filename, thumb_path, mime_type, encrypted_blob_id, user_id) in &photos {
        let is_encrypted = encrypted_blob_id.is_some();

        // Determine if this format needs a web preview at all
        let preview_ext = match needs_web_preview(filename) {
            Some(ext) => ext,
            None => {
                // No conversion needed for this format.
                // Still check plain photo thumbnails below.
                if !is_encrypted {
                    if let Some(tp) = thumb_path {
                        let thumb_abs = storage_root.join(tp);
                        if !tokio::fs::try_exists(&thumb_abs).await.unwrap_or(false) {
                            let source_path = storage_root.join(file_path);
                            if tokio::fs::try_exists(&source_path).await.unwrap_or(false) {
                                if generate_thumbnail_file(&source_path, &thumb_abs, mime_type, None).await {
                                    thumbnails_generated += 1;
                                    tracing::debug!(
                                        photo_id = %photo_id,
                                        "Background convert: generated missing thumbnail"
                                    );
                                }
                            }
                        }
                    }
                }
                continue;
            }
        };

        // ── Path A: Source file exists on disk → convert normally ────────
        let source_path = storage_root.join(file_path);
        if tokio::fs::try_exists(&source_path).await.unwrap_or(false) {
            let preview_path = storage_root.join(format!(
                ".web_previews/{}.web.{}",
                photo_id, preview_ext
            ));
            if !tokio::fs::try_exists(&preview_path).await.unwrap_or(false) {
                tracing::info!(
                    photo_id = %photo_id,
                    filename = %filename,
                    target_ext = preview_ext,
                    encrypted = is_encrypted,
                    "Background convert: starting conversion (source on disk)"
                );

                if generate_web_preview_bg(&source_path, &preview_path, preview_ext).await {
                    converted += 1;
                    tracing::info!(
                        photo_id = %photo_id,
                        filename = %filename,
                        "Background convert: conversion complete (source on disk)"
                    );

                    // If this photo is encrypted AND we have the key, also
                    // re-encrypt the blob with the converted data so the
                    // client gets web-compatible media after decryption.
                    if is_encrypted && key_available {
                        if let Err(e) = reencrypt_blob_with_converted_data(
                            pool, storage_root, encryption_key, photo_id, user_id,
                            encrypted_blob_id.as_deref().unwrap(), filename, &preview_path, preview_ext,
                        ).await {
                            tracing::warn!(
                                photo_id = %photo_id,
                                "Background convert: re-encryption failed: {}", e
                            );
                        }
                    }
                } else {
                    tracing::warn!(
                        photo_id = %photo_id,
                        filename = %filename,
                        "Background convert: conversion failed (source on disk)"
                    );
                }

                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            } else if is_encrypted {
                // Preview already exists on disk AND photo is encrypted.
                // The migration already put web-compatible data in the blob,
                // so mark this photo as blob-converted to prevent false
                // "pending" counts in conversion_status().
                mark_blob_converted(pool, photo_id).await;
                tracing::debug!(
                    photo_id = %photo_id,
                    "Background convert: marked encrypted photo as converted (preview already on disk)"
                );
            }

            // Generate thumbnail if missing — only for plain photos.
            if !is_encrypted {
                if let Some(tp) = thumb_path {
                    let thumb_abs = storage_root.join(tp);
                    if !tokio::fs::try_exists(&thumb_abs).await.unwrap_or(false) {
                        if generate_thumbnail_file(&source_path, &thumb_abs, mime_type, None).await {
                            thumbnails_generated += 1;
                            tracing::debug!(
                                photo_id = %photo_id,
                                "Background convert: generated missing thumbnail"
                            );
                        }
                    }
                }
            }
            continue;
        }

        // ── Path B: No source file on disk, but encrypted → temp-decrypt ─
        if is_encrypted && key_available {
            let blob_id = encrypted_blob_id.as_deref().unwrap();

            // Check if this blob already contains converted data by looking
            // at whether we previously marked it as converted.
            let already_converted = is_blob_already_converted(pool, photo_id).await;
            if already_converted {
                continue;
            }

            tracing::info!(
                photo_id = %photo_id,
                filename = %filename,
                target_ext = preview_ext,
                "Background convert: starting conversion (temp-decrypt encrypted blob)"
            );

            match decrypt_convert_reencrypt(
                pool, storage_root, encryption_key, photo_id, user_id,
                blob_id, filename, preview_ext,
            ).await {
                Ok(true) => {
                    converted += 1;
                    tracing::info!(
                        photo_id = %photo_id,
                        filename = %filename,
                        "Background convert: conversion complete (re-encrypted blob)"
                    );
                }
                Ok(false) => {
                    tracing::warn!(
                        photo_id = %photo_id,
                        filename = %filename,
                        "Background convert: conversion failed (encrypted blob)"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        photo_id = %photo_id,
                        filename = %filename,
                        "Background convert: decrypt/convert/reencrypt error: {}", e
                    );
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    if converted > 0 || thumbnails_generated > 0 {
        tracing::info!(
            "Background convert: converted {} files, generated {} thumbnails this cycle",
            converted, thumbnails_generated
        );
    }

    (converted, thumbnails_generated)
}

/// Check if an encrypted blob has already been converted by looking at
/// the mime_type stored in the encrypted payload. We track this with a
/// simple check: if a `.web_previews/{id}.converted` marker file exists,
/// the blob has been re-encrypted with converted data.
async fn is_blob_already_converted(pool: &SqlitePool, photo_id: &str) -> bool {
    // Use a lightweight marker approach: check for a sentinel file
    // that we create after successful re-encryption.
    // Alternatively, we could store this in the DB, but a marker file
    // is simpler and doesn't require a schema migration.
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
    let _ = sqlx::query(
        "INSERT OR REPLACE INTO server_settings (key, value) VALUES (?, 'true')",
    )
    .bind(format!("blob_converted_{}", photo_id))
    .execute(pool)
    .await;
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
    filename: &str,
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

/// Run the background conversion loop.
/// Checks for unconverted files every `interval_secs` seconds, or immediately
/// when notified via the `notify` handle.
///
/// The `active` flag is set while the converter has pending work, allowing the
/// conversion-status endpoint to keep the client banner alive even if
/// encryption changes the DB state mid-pass.
///
/// The `encryption_key` handle is checked each pass — when available, the
/// converter can process encrypted blobs by decrypting, converting, and
/// re-encrypting.
pub async fn background_convert_task(
    pool: SqlitePool,
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

        // Mark active before the pass so the banner shows immediately
        active.store(true, Ordering::Release);

        let (converted, thumbs) = run_conversion_pass(&pool, &storage_root, &encryption_key).await;
        tracing::info!(
            "[DIAG:CONVERT] pass result: converted={}, thumbs={}, setting active={}",
            converted, thumbs, converted > 0 || thumbs > 0
        );

        if converted > 0 || thumbs > 0 {
            // If we converted any files, run a second pass to generate
            // thumbnails for the newly converted items (the web preview
            // may now be usable as a thumbnail source).
            if converted > 0 {
                tracing::info!("Background convert: running second pass for thumbnails of converted files");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                run_conversion_pass(&pool, &storage_root, &encryption_key).await;
            }
        }

        // Always clear the active flag after a pass completes.
        active.store(false, Ordering::Release);
        tracing::info!("[DIAG:CONVERT] active flag cleared to false");
    }
}

/// Admin endpoint: trigger an immediate background conversion pass.
///
/// POST /admin/photos/convert
pub async fn trigger_convert(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Verify admin role
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&_auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

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
/// were not converted at the time (e.g. the converter was previously skipping
/// during migration, or no FFmpeg was available). The key is stored temporarily
/// in `AppState` for 30 minutes, during which the background converter will
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
    // Verify admin role
    let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await?;
    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

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
        let storage_root = state.storage_root.read().await.clone();
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

    // Count how many encrypted photos need conversion
    let photos: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, filename FROM photos WHERE encrypted_blob_id IS NOT NULL",
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

/// Check how many files still need conversion or thumbnails for the current user.
///
/// GET /photos/conversion-status
///
/// Counts photos needing web-preview conversion and thumbnails.
/// For encrypted photos, counts those whose blobs have not yet been
/// re-encrypted with web-compatible data (tracked via `blob_converted_`
/// markers in server_settings).
pub async fn conversion_status(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let storage_root = state.storage_root.read().await.clone();
    let converting = state.conversion_active.load(Ordering::Acquire);
    let key_available = state.encryption_key.read().await.is_some();

    // ── Check encryption mode ───────────────────────────────────────────
    let enc_mode: String = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_mode'",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "plain".to_string());

    let mig_status: String = sqlx::query_scalar(
        "SELECT status FROM encryption_migration WHERE id = 'singleton'",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "idle".to_string());

    // ── Check ALL photos for pending web-preview conversions ───────────
    let photos: Vec<(String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, filename, file_path, thumb_path, encrypted_blob_id FROM photos WHERE user_id = ?",
    )
    .bind(&_auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let mut pending_conversions = 0u32;   // Items the converter CAN process now
    let mut pending_awaiting_key = 0u32;   // Encrypted items needing key first
    let mut missing_thumbnails = 0u32;
    let migration_running = mig_status == "encrypting" || mig_status == "decrypting";

    for (id, filename, file_path, thumb_path, encrypted_blob_id) in &photos {
        let is_encrypted = encrypted_blob_id.is_some();

        // Check if format needs a web preview that hasn't been generated
        if let Some(_ext) = super::scan::needs_web_preview(filename) {
            if is_encrypted {
                let already_converted = is_blob_already_converted(&state.pool, id).await;
                if !already_converted {
                    // Also check if a disk-based preview already exists.
                    // If it does, the migration already embedded the converted
                    // data in the encrypted blob, so this item is effectively done.
                    let disk_preview = storage_root.join(format!(".web_previews/{}.web.{}", id, _ext));
                    let preview_on_disk = tokio::fs::try_exists(&disk_preview).await.unwrap_or(false);
                    if preview_on_disk {
                        // Preview exists → blob already has converted data; skip
                    } else if key_available && !migration_running {
                        // Key is available AND migration not running → converter can process this
                        pending_conversions += 1;
                    } else {
                        // Can't convert yet: either no key or migration is still running
                        pending_awaiting_key += 1;
                    }
                }
            } else {
                // For plain photos: check if the preview file exists on disk
                let preview_path =
                    storage_root.join(format!(".web_previews/{}.web.{}", id, _ext));
                if !tokio::fs::try_exists(&preview_path).await.unwrap_or(false) {
                    let source_exists = tokio::fs::try_exists(storage_root.join(file_path))
                        .await
                        .unwrap_or(false);
                    if source_exists {
                        pending_conversions += 1;
                    }
                }
            }
        }

        // Check if thumbnail is missing (plain photos only).
        if !is_encrypted {
            if let Some(tp) = thumb_path {
                let thumb_abs = storage_root.join(tp);
                if !tokio::fs::try_exists(&thumb_abs).await.unwrap_or(false) {
                    missing_thumbnails += 1;
                }
            }
        }
    }

    // ── Encrypted-mode: count photos missing encrypted thumbnail blobs ──
    let enc_missing_thumbs: i64 = if enc_mode == "encrypted" {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NOT NULL AND encrypted_thumb_blob_id IS NULL AND user_id = ?",
        )
        .bind(&_auth.user_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0)
    } else {
        0
    };

    // Don't count encrypted missing thumbs during migration — they're being
    // generated as part of the migration process, not conversion.
    if migration_running {
        // During migration the thumbnail blobs get created alongside encryption.
        // Don't conflate that with conversion progress.
    } else if enc_missing_thumbs > 0 {
        missing_thumbnails += enc_missing_thumbs as u32;
    }

    // Only log STATUS when there's something interesting
    if pending_conversions > 0 || pending_awaiting_key > 0 || missing_thumbnails > 0 || converting || enc_missing_thumbs > 0 {
        tracing::info!(
            "[DIAG:STATUS] conversion-status: pending={}, awaiting_key={}, thumbs={}, converting={}, total_photos={}, enc_mode={}, mig_status={}, enc_missing_thumbs={}, key_available={}",
            pending_conversions, pending_awaiting_key, missing_thumbnails, converting, photos.len(), enc_mode, mig_status, enc_missing_thumbs, key_available
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
