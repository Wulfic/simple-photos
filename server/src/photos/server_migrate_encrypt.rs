//! Single-photo encryption pipeline and thumbnail repair for the migration.
//!
//! Handles:
//! - Reading a plain photo from disk
//! - Generating web previews and thumbnails  
//! - Encrypting payloads with AES-256-GCM
//! - Writing encrypted blobs and updating the DB
//! - Post-migration repair for photos missing encrypted thumbnails

use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::blobs::storage;
use crate::crypto;

use super::thumbnail::{apply_exif_orientation_from_bytes, generate_thumbnail_file};
use super::web_preview::{generate_web_preview_bg, needs_web_preview};

// ── Data model ───────────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
pub struct PlainPhotoRow {
    pub id: String,
    pub user_id: String,
    pub filename: String,
    pub file_path: String,
    pub mime_type: String,
    pub media_type: String,
    #[allow(dead_code)]
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub created_at: String,
}

// ── Thumbnail generation for migration ───────────────────────────────────────

/// Generate a 256×256 JPEG thumbnail for the migration pipeline.
///
/// Multi-stage fallback:
/// 1. Audio → black 256×256 placeholder.
/// 2. Non-video images → `image::load_from_memory` first (fast, in-memory).
/// 3. Fallback → `generate_thumbnail_file` (FFmpeg/ImageMagick).
pub async fn generate_thumbnail_for_migration(
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
                // Apply EXIF orientation so portrait photos are thumbnailed
                // correctly (image::load_from_memory ignores EXIF).
                let img = apply_exif_orientation_from_bytes(&data_owned, img);
                let thumb = img.resize(512, 512, image::imageops::FilterType::Triangle);
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

    // Fallback: generate_thumbnail_file (FFmpeg/ImageMagick)
    // UUID v4 filename prevents predictable temp file attacks.
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

// ── Single-photo encryption pipeline ─────────────────────────────────────────

/// Encrypt one photo: read from disk → web preview → thumbnail → encrypt → write blobs → update DB.
pub async fn encrypt_one_photo(
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

    // Check if a blob with this content already exists (e.g. synced from primary).
    // If so, reuse it instead of re-encrypting and creating a duplicate blob.
    let content_hash = hex::encode(&Sha256::digest(&file_data)[..6]);
    let existing_blob: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM blobs WHERE content_hash = ? AND user_id = ? LIMIT 1",
    )
    .bind(&content_hash)
    .bind(&photo.user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Check existing blob: {}", e))?;

    if let Some((existing_blob_id,)) = existing_blob {
        // Find the encrypted thumbnail that is already associated with the
        // photo that owns the matched blob.  This avoids picking a random
        // unlinked thumbnail which could belong to a completely different
        // photo (e.g. when a secure-gallery clone shares the same
        // content_hash as the original).
        let thumb_type = if photo.media_type == "video" {
            "video_thumbnail"
        } else {
            "thumbnail"
        };
        let existing_thumb_id: Option<String> = sqlx::query_scalar(
            "SELECT p.encrypted_thumb_blob_id \
             FROM photos p \
             WHERE p.encrypted_blob_id = ? AND p.user_id = ? \
               AND p.encrypted_thumb_blob_id IS NOT NULL \
             LIMIT 1",
        )
        .bind(&existing_blob_id)
        .bind(&photo.user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Check existing thumb: {}", e))?
        .flatten();

        // Fallback: if no photo references the blob yet (backup-sync scenario),
        // look for an unlinked thumbnail of the right type.
        let existing_thumb_id = if existing_thumb_id.is_some() {
            existing_thumb_id
        } else {
            sqlx::query_scalar(
                "SELECT id FROM blobs \
                 WHERE user_id = ? AND blob_type = ? \
                   AND id NOT IN (SELECT encrypted_thumb_blob_id FROM photos WHERE encrypted_thumb_blob_id IS NOT NULL) \
                 ORDER BY upload_time ASC LIMIT 1",
            )
            .bind(&photo.user_id)
            .bind(thumb_type)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("Check existing thumb fallback: {}", e))?
        };

        tracing::info!(
            "[SERVER_MIG] reusing existing synced blob {} for {} (content_hash={})",
            existing_blob_id,
            photo.filename,
            content_hash
        );

        sqlx::query(
            "UPDATE photos SET encrypted_blob_id = ?, encrypted_thumb_blob_id = ? WHERE id = ? AND user_id = ?",
        )
        .bind(&existing_blob_id)
        .bind(existing_thumb_id.as_deref())
        .bind(&photo.id)
        .bind(&photo.user_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Link existing blob failed: {}", e))?;

        return Ok(());
    }

    // Generate web preview and thumbnail concurrently
    let web_preview_fut = build_web_preview(&photo, &full_path, &file_data, storage_root);
    let thumbnail_fut = build_thumbnail(&photo, &full_path, &file_data, storage_root);

    let ((payload_data, payload_mime), thumb_data) =
        tokio::join!(web_preview_fut, thumbnail_fut);

    // Upload thumbnail blob (if generated)
    let mut thumb_blob_id = String::new();
    let thumb_insert_params = if let Some(ref thumb_bytes) = thumb_data {
        let params = encrypt_and_write_thumbnail(
            thumb_bytes,
            key,
            &photo,
            storage_root,
        )
        .await?;
        thumb_blob_id = params.0.clone();
        Some(params)
    } else {
        None
    };

    // Build the photo payload JSON (same format as the client)
    let server_blob_type = classify_blob_type(&photo.mime_type);

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

// ── Repair pass ──────────────────────────────────────────────────────────────

/// Fix photos that were encrypted but are missing their encrypted thumbnail.
pub async fn repair_missing_thumbnails(
    key: [u8; 32],
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
) {
    let missing_thumbs: Vec<PlainPhotoRow> = match sqlx::query_as::<_, PlainPhotoRow>(
        "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, created_at \
         FROM photos WHERE encrypted_blob_id IS NOT NULL AND encrypted_thumb_blob_id IS NULL",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("[SERVER_MIG] repair pass query failed: {}", e);
            return;
        }
    };

    if missing_thumbs.is_empty() {
        return;
    }

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
            match encrypt_and_store_repair_thumbnail(
                &thumb_bytes,
                &key,
                &photo,
                pool,
                storage_root,
                &photo_id,
                &photo_user_id,
            )
            .await
            {
                Ok(()) => {
                    tracing::info!(
                        "[SERVER_MIG] repair: generated encrypted thumbnail for {}",
                        filename
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "[SERVER_MIG] repair: failed for {}: {}",
                        filename,
                        e
                    );
                }
            }
        }
    }
}

/// One-time repair: regenerate encrypted thumbnail blobs for photos whose
/// source image has EXIF orientation ≥ 2.
///
/// The original migration (`generate_thumbnail_for_migration`) did not apply
/// EXIF orientation, so encrypted thumbnail blobs for portrait camera photos
/// contain landscape-oriented pixel data.  This task re-generates those
/// thumbnails with correct orientation, re-encrypts them, and replaces the
/// old blob reference.
///
/// Requires the encryption key (loaded from the wrapped key in the DB).
pub async fn repair_encrypted_thumbnail_orientation(
    key: [u8; 32],
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
) {
    // Check one-time flag
    let done: Option<String> = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'enc_thumb_orientation_repaired'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    if done.is_some() {
        return;
    }

    // Find all encrypted photos that have both a source file and an encrypted thumbnail
    let rows: Vec<PlainPhotoRow> = match sqlx::query_as::<_, PlainPhotoRow>(
        "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, created_at \
         FROM photos \
         WHERE file_path != '' AND encrypted_thumb_blob_id IS NOT NULL \
         AND media_type = 'photo' AND mime_type != 'image/gif'",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("[ENC_THUMB_REPAIR] Failed to query photos: {}", e);
            return;
        }
    };

    if rows.is_empty() {
        tracing::info!("[ENC_THUMB_REPAIR] No photos to check");
    } else {
        let mut repaired = 0u64;
        let total = rows.len();

        for photo in &rows {
            let full_path = storage_root.join(&photo.file_path);
            if !full_path.exists() {
                continue;
            }

            // Check EXIF orientation — only repair if rotation/flip is needed
            let path_clone = full_path.clone();
            let orientation = tokio::task::spawn_blocking(move || {
                (|| -> Option<u32> {
                    let file = std::fs::File::open(&path_clone).ok()?;
                    let mut reader = std::io::BufReader::new(file);
                    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
                    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
                    field.value.get_uint(0)
                })()
                .unwrap_or(0)
            })
            .await
            .unwrap_or(0);

            if orientation < 2 {
                continue; // No rotation needed
            }

            // Re-generate thumbnail with correct EXIF orientation
            let file_data = match tokio::fs::read(&full_path).await {
                Ok(d) => d,
                Err(_) => continue,
            };

            let thumb_data =
                generate_thumbnail_for_migration(&full_path, &file_data, &photo.mime_type).await;

            if let Some(thumb_bytes) = thumb_data {
                match encrypt_and_store_repair_thumbnail(
                    &thumb_bytes,
                    &key,
                    photo,
                    pool,
                    storage_root,
                    &photo.id,
                    &photo.user_id,
                )
                .await
                {
                    Ok(()) => {
                        repaired += 1;
                        tracing::debug!(
                            photo_id = %photo.id,
                            orientation = orientation,
                            "[ENC_THUMB_REPAIR] Regenerated encrypted thumbnail"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            photo_id = %photo.id,
                            error = %e,
                            "[ENC_THUMB_REPAIR] Failed to repair"
                        );
                    }
                }
            }
        }

        tracing::info!(
            "[ENC_THUMB_REPAIR] Checked {} photos, regenerated {} encrypted thumbnails",
            total,
            repaired
        );
    }

    // Record flag
    let _ = sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('enc_thumb_orientation_repaired', '1') \
         ON CONFLICT(key) DO UPDATE SET value = '1'",
    )
    .execute(pool)
    .await;
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Build a web preview for non-browser-native formats, falling back to the
/// original file data if no conversion is needed.
async fn build_web_preview(
    photo: &PlainPhotoRow,
    full_path: &std::path::Path,
    file_data: &[u8],
    storage_root: &std::path::Path,
) -> (Vec<u8>, String) {
    if let Some(preview_ext) = needs_web_preview(&photo.filename) {
        let cached_preview_path = storage_root.join(format!(
            ".web_previews/{}.web.{}",
            photo.id, preview_ext
        ));

        let preview_data = if tokio::fs::try_exists(&cached_preview_path)
            .await
            .unwrap_or(false)
        {
            tokio::fs::read(&cached_preview_path).await.ok()
        } else {
            tracing::info!(
                "[SERVER_MIG] Generating web preview for {} before encryption",
                photo.filename
            );
            if generate_web_preview_bg(full_path, &cached_preview_path, preview_ext).await {
                tokio::fs::read(&cached_preview_path).await.ok()
            } else {
                None
            }
        };

        if let Some(data) = preview_data {
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
    (file_data.to_vec(), photo.mime_type.clone())
}

/// Build a thumbnail, using cached version if available.
async fn build_thumbnail(
    photo: &PlainPhotoRow,
    full_path: &std::path::Path,
    file_data: &[u8],
    storage_root: &std::path::Path,
) -> Option<Vec<u8>> {
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
                return generate_thumbnail_for_migration(full_path, file_data, &photo.mime_type)
                    .await;
            }
            return cached;
        }
        return generate_thumbnail_for_migration(full_path, file_data, &photo.mime_type)
            .await;
    }
    generate_thumbnail_for_migration(full_path, file_data, &photo.mime_type).await
}

/// Encrypt thumbnail bytes and write to storage. Returns params for DB insert.
async fn encrypt_and_write_thumbnail(
    thumb_bytes: &[u8],
    key: &[u8; 32],
    photo: &PlainPhotoRow,
    storage_root: &std::path::Path,
) -> Result<(String, String, i64, String, String, String), String> {
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

    let now = Utc::now().to_rfc3339();
    Ok((
        blob_id,
        blob_type.to_string(),
        enc_thumb.len() as i64,
        enc_thumb_hash,
        now,
        blob_storage_path,
    ))
}

/// Classify the blob type based on MIME type.
fn classify_blob_type(mime_type: &str) -> &'static str {
    if mime_type == "image/gif" {
        "gif"
    } else if mime_type.starts_with("video/") {
        "video"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else {
        "photo"
    }
}

/// Encrypt and store a repair thumbnail, inserting the blob row and updating
/// the photos table in one go.
async fn encrypt_and_store_repair_thumbnail(
    thumb_bytes: &[u8],
    key: &[u8; 32],
    photo: &PlainPhotoRow,
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
    photo_id: &str,
    photo_user_id: &str,
) -> Result<(), String> {
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
            .map_err(|e| format!("Encrypt failed: {}", e))?
    };

    let enc_thumb_hash = hex::encode(Sha256::digest(&enc_thumb));
    let blob_id = Uuid::new_v4().to_string();
    let blob_type = if photo.media_type == "video" {
        "video_thumbnail"
    } else {
        "thumbnail"
    };

    let blob_storage_path =
        storage::write_blob(storage_root, photo_user_id, &blob_id, &enc_thumb)
            .await
            .map_err(|e| format!("Write blob failed: {}", e))?;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&blob_id)
    .bind(photo_user_id)
    .bind(blob_type)
    .bind(enc_thumb.len() as i64)
    .bind(&enc_thumb_hash)
    .bind(&now)
    .bind(&blob_storage_path)
    .execute(pool)
    .await
    .map_err(|e| format!("Insert blob failed: {}", e))?;

    sqlx::query(
        "UPDATE photos SET encrypted_thumb_blob_id = ? WHERE id = ? AND user_id = ?",
    )
    .bind(&blob_id)
    .bind(photo_id)
    .bind(photo_user_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Update photos failed: {}", e))?;

    Ok(())
}
