//! Rendered save-copy — the "Save As Copy" action in the editing engine.
//!
//! `POST /api/photos/:id/duplicate` creates a fully independent rendered copy
//! of the photo.  When `crop_metadata` is provided, edits are baked into the
//! new file via **ffmpeg** (video/audio) or the **image crate** (photos),
//! producing a new `photos` row with `crop_metadata = NULL`.
//!
//! When no crop_metadata is given the original file is copied verbatim so the
//! duplicate is still a fully independent file on disk.

use std::path::Path as StdPath;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::blobs::storage;
use crate::crypto;
use crate::error::AppError;
use crate::photos::metadata::extract_media_metadata_async;
use crate::photos::models::Photo;
use crate::photos::thumbnail::{generate_thumbnail_file, probe_duration};
use crate::state::AppState;

use super::models::{validate_crop_json, CropMeta};

/// Request body for `POST /api/photos/{id}/duplicate`.
///
/// When `crop_metadata` is provided the edits are baked into a new rendered
/// file; the copy's `crop_metadata` will be `NULL`.
#[derive(Debug, Deserialize)]
pub struct DuplicatePhotoRequest {
    pub crop_metadata: Option<String>,
}

/// POST /api/photos/:id/duplicate — render a fully independent copy.
///
/// When `crop_metadata` is supplied, edits are applied via ffmpeg (video/audio)
/// or the image crate (images) and baked into a new file.  The resulting
/// `photos` row has its own `file_path`, `thumb_path`, correct dimensions,
/// and `crop_metadata = NULL`.
///
/// When no crop_metadata is given, the original file is copied verbatim.
pub async fn duplicate_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(photo_id): Path<String>,
    Json(req): Json<DuplicatePhotoRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // ── Fetch original ───────────────────────────────────────────────────
    let original: Option<Photo> = sqlx::query_as(
        "SELECT id, user_id, filename, file_path, mime_type, media_type, size_bytes, \
         width, height, duration_secs, taken_at, latitude, longitude, thumb_path, \
         created_at, encrypted_blob_id, encrypted_thumb_blob_id, is_favorite, \
         crop_metadata, camera_model, photo_hash \
         FROM photos WHERE id = ? AND user_id = ?",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let original = original.ok_or(AppError::NotFound)?;

    tracing::info!(
        "[editing/save_copy] Starting duplicate for photo_id={}, user={}, \
         original={}×{}, media_type={}, mime={}, has_crop_metadata={}",
        photo_id,
        auth.user_id,
        original.width,
        original.height,
        original.media_type,
        original.mime_type,
        req.crop_metadata.is_some(),
    );

    if let Some(ref raw_crop) = req.crop_metadata {
        tracing::info!(
            "[editing/save_copy] Raw crop_metadata from client: {}",
            raw_crop,
        );
    }

    // ── Validate crop_metadata JSON if provided ──────────────────────────
    let meta_json: Option<String> = req
        .crop_metadata
        .as_deref()
        .map(|m| validate_crop_json(m, 2048))
        .transpose()?;

    let meta: Option<CropMeta> = meta_json.as_deref().and_then(CropMeta::from_json);

    if let Some(ref m) = meta {
        tracing::info!(
            "[editing/save_copy] Parsed CropMeta: crop=({:.4},{:.4},{:.4},{:.4}), \
             rotate={}°, brightness={:.1}, trim=({:.2},{:.2}), \
             has_crop={}, has_rotation={}, swaps_dims={}",
            m.x.unwrap_or(0.0), m.y.unwrap_or(0.0),
            m.width.unwrap_or(1.0), m.height.unwrap_or(1.0),
            m.rotation_degrees(),
            m.brightness.unwrap_or(0.0),
            m.trim_start.unwrap_or(0.0), m.trim_end.unwrap_or(0.0),
            m.has_crop(), m.has_rotation(), m.rotation_swaps_dimensions(),
        );
    }

    let new_id = Uuid::new_v4().to_string();

    // ── Prepare output path ──────────────────────────────────────────────
    let storage_root = (**state.storage_root.load()).clone();
    let uploads_dir = storage_root.join("uploads");
    tokio::fs::create_dir_all(&uploads_dir).await.map_err(|e| {
        AppError::Internal(format!("Failed to create uploads directory: {e}"))
    })?;

    let ext = StdPath::new(&original.filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let copy_disk_name = format!("copy-{}.{}", new_id, ext);
    let copy_abs = uploads_dir.join(&copy_disk_name);
    let copy_rel = format!("uploads/{}", copy_disk_name);

    // ── Build "Copy of <filename>" display name ──────────────────────────
    let copy_filename = if original.filename.starts_with("Copy of ") {
        original.filename.clone()
    } else {
        format!("Copy of {}", original.filename)
    };

    let source_abs = storage_root.join(&original.file_path);

    // For blob-only photos (uploaded from mobile, no file on disk), decrypt
    // the encrypted blob to a temp file so the rendering pipeline can read it.
    let temp_source_path: Option<std::path::PathBuf>;
    let source_abs = if original.file_path.is_empty() {
        if original.encrypted_blob_id.is_empty() {
            return Err(AppError::Internal("Photo has no file_path and no encrypted_blob_id".into()));
        }
        let blob_id = &original.encrypted_blob_id;
        tracing::info!(
            "[editing/save_copy] Blob-only photo — decrypting blob {} to temp file",
            blob_id,
        );
        let enc_key = crypto::load_wrapped_key(&state.pool, &state.config.auth.jwt_secret)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to load encryption key: {e}")))?
            .ok_or_else(|| AppError::Internal("No encryption key configured".into()))?;

        // Look up the blob's storage_path
        let blob_row: Option<(String,)> = sqlx::query_as(
            "SELECT storage_path FROM blobs WHERE id = ? AND user_id = ?",
        )
        .bind(blob_id)
        .bind(&auth.user_id)
        .fetch_optional(&state.read_pool)
        .await?;
        let (blob_storage_path,) = blob_row.ok_or(AppError::NotFound)?;

        let enc_data = storage::read_blob(&storage_root, &blob_storage_path).await?;
        let plaintext = {
            let k = enc_key;
            tokio::task::spawn_blocking(move || crypto::decrypt(&k, &enc_data))
                .await
                .map_err(|e| AppError::Internal(format!("Decrypt panicked: {e}")))?
                .map_err(|e| AppError::Internal(format!("Decrypt failed: {e}")))?
        };

        // Parse JSON envelope and extract the base64 "data" field
        let envelope: serde_json::Value = serde_json::from_slice(&plaintext)
            .map_err(|e| AppError::Internal(format!("Invalid blob envelope JSON: {e}")))?;
        let data_b64 = envelope["data"]
            .as_str()
            .ok_or_else(|| AppError::Internal("Missing 'data' field in blob envelope".into()))?;
        let raw_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            data_b64,
        )
        .map_err(|e| AppError::Internal(format!("Base64 decode failed: {e}")))?;

        // Write to a temp file with proper extension for ffmpeg/image crate
        let tmp_path = std::env::temp_dir().join(format!("sp-dup-{}.{}", new_id, ext));
        tokio::fs::write(&tmp_path, &raw_bytes).await.map_err(|e| {
            AppError::Internal(format!("Failed to write temp source: {e}"))
        })?;
        tracing::info!(
            "[editing/save_copy] Decrypted blob to temp file: {} ({} bytes)",
            tmp_path.display(),
            raw_bytes.len(),
        );
        temp_source_path = Some(tmp_path.clone());
        tmp_path
    } else {
        temp_source_path = None;
        if !tokio::fs::try_exists(&source_abs).await.unwrap_or(false) {
            return Err(AppError::NotFound);
        }
        source_abs.clone()
    };

    let media_type = original.media_type.as_str();
    let has_edits = meta.is_some();

    // ── Render or copy the file ──────────────────────────────────────────
    if has_edits && (media_type == "video" || media_type == "audio") {
        tracing::info!(
            "[editing/save_copy] Rendering video/audio via ffmpeg: {} → {}",
            source_abs.display(), copy_abs.display(),
        );
        super::ffmpeg::run_ffmpeg_render(
            &source_abs,
            &copy_abs,
            media_type,
            meta.as_ref().unwrap(),
            ext,
        )
        .await?;
        tracing::info!("[editing/save_copy] FFmpeg render completed");
    } else if has_edits && media_type == "photo" {
        tracing::info!(
            "[editing/save_copy] Rendering photo via image crate: {} → {}",
            source_abs.display(), copy_abs.display(),
        );
        super::image_render::render_image(
            &source_abs,
            &copy_abs,
            meta.as_ref().unwrap(),
        )
        .await?;
        tracing::info!("[editing/save_copy] Image render completed");
    } else {
        tracing::info!(
            "[editing/save_copy] No edits — plain file copy: {} → {}",
            source_abs.display(), copy_abs.display(),
        );
        tokio::fs::copy(&source_abs, &copy_abs).await.map_err(|e| {
            AppError::Internal(format!("Failed to copy file: {e}"))
        })?;
    }

    // Clean up decrypted temp source file (blob-only photos)
    if let Some(ref tmp) = temp_source_path {
        let _ = tokio::fs::remove_file(tmp).await;
    }

    // ── Probe rendered file for dimensions and size ──────────────────────
    let file_meta = tokio::fs::metadata(&copy_abs).await.map_err(|e| {
        AppError::Internal(format!("Failed to stat rendered copy: {e}"))
    })?;
    let size_bytes = file_meta.len() as i64;

    let (new_w, new_h, _, _, _, _) =
        extract_media_metadata_async(copy_abs.clone()).await;

    tracing::info!(
        "[editing/save_copy] Rendered file metadata: {}×{}, size={} bytes, \
         original was {}×{}",
        new_w, new_h, size_bytes,
        original.width, original.height,
    );

    // Sanity check: if rotation swaps dimensions, verify they actually changed
    if let Some(ref m) = meta {
        if m.rotation_swaps_dimensions() {
            let expected_swap = (new_w != original.width) || (new_h != original.height);
            tracing::info!(
                "[editing/save_copy] Rotation {}° swaps dims: original={}×{} → \
                 rendered={}×{} (dims_changed={})",
                m.rotation_degrees(), original.width, original.height,
                new_w, new_h, expected_swap,
            );
        }
    }

    // For video copies, probe the new duration
    let new_duration = if media_type == "video" || media_type == "audio" {
        probe_duration(&copy_abs).await
    } else {
        None
    };

    // ── Generate thumbnail from the rendered file ────────────────────────
    let thumb_ext = if original.mime_type == "image/gif" {
        "gif"
    } else {
        "jpg"
    };
    let thumb_rel = format!(".thumbnails/{}.thumb.{}", new_id, thumb_ext);
    let thumb_abs = storage_root.join(&thumb_rel);
    let thumb_rel_opt = {
        let mime_clone = original.mime_type.clone();
        let copy_abs_c = copy_abs.clone();
        let thumb_abs_c = thumb_abs.clone();
        tracing::info!(
            "[editing/save_copy] Generating thumbnail from rendered file: {} → {}",
            copy_abs_c.display(), thumb_abs_c.display(),
        );
        let ok = generate_thumbnail_file(&copy_abs_c, &thumb_abs_c, &mime_clone, None).await;
        if ok {
            // Log the actual thumbnail dimensions
            if let Ok(tsize) = imagesize::size(&thumb_abs_c) {
                tracing::info!(
                    "[editing/save_copy] Thumbnail generated: {}×{} (source rendered={}×{})",
                    tsize.width, tsize.height, new_w, new_h,
                );
            }
            Some(thumb_rel.clone())
        } else {
            tracing::warn!(
                "[editing/save_copy] Thumbnail generation FAILED for {}",
                copy_abs_c.display(),
            );
            None
        }
    };

    // ── Use the original's taken_at (for timeline ordering) ──────────────
    let created_at = original.created_at.clone();

    // ── Try to encrypt the copy inline (so no unencrypted file persists) ─
    let enc_key = crypto::load_wrapped_key(&state.pool, &state.config.auth.jwt_secret)
        .await
        .ok()
        .flatten();

    let (_final_file_path, _final_thumb_path, enc_blob_id, enc_thumb_blob_id) =
        if let Some(key) = enc_key {
            encrypt_and_store_copy(
                &state,
                &auth,
                &original,
                key,
                &new_id,
                &copy_filename,
                &copy_abs,
                &thumb_abs,
                thumb_rel_opt.as_deref(),
                &storage_root,
                &created_at,
                new_w,
                new_h,
                new_duration,
                size_bytes,
            )
            .await?
        } else {
            // No encryption key — fall back to unencrypted storage
            sqlx::query(
                "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
                 size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
                 thumb_path, created_at, encrypted_blob_id, encrypted_thumb_blob_id, \
                 is_favorite, crop_metadata, camera_model, photo_hash) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, 0, NULL, ?, NULL)",
            )
            .bind(&new_id)
            .bind(&auth.user_id)
            .bind(&copy_filename)
            .bind(&copy_rel)
            .bind(&original.mime_type)
            .bind(&original.media_type)
            .bind(size_bytes)
            .bind(new_w)
            .bind(new_h)
            .bind(new_duration.or(original.duration_secs))
            .bind(&original.taken_at)
            .bind(original.latitude)
            .bind(original.longitude)
            .bind(&thumb_rel_opt)
            .bind(&created_at)
            .bind(&original.camera_model)
            .execute(&state.pool)
            .await?;

            (
                copy_rel.clone(),
                thumb_rel_opt.clone(),
                Option::<String>::None,
                Option::<String>::None,
            )
        };

    tracing::info!(
        "Rendered duplicate {} → {} ({}×{}, encrypted={}) for user {}",
        photo_id,
        new_id,
        new_w,
        new_h,
        enc_blob_id.is_some(),
        auth.user_id
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": new_id,
            "source_photo_id": photo_id,
            "filename": copy_filename,
            "crop_metadata": null,
            "width": new_w,
            "height": new_h,
            "duration_secs": new_duration.or(original.duration_secs),
            "mime_type": original.mime_type,
            "media_type": original.media_type,
            "size_bytes": size_bytes,
            "encrypted_blob_id": enc_blob_id,
            "encrypted_thumb_blob_id": enc_thumb_blob_id,
        })),
    ))
}

// ── Inline encryption helper ─────────────────────────────────────────────────

/// Encrypt the rendered copy and its thumbnail, store them as blobs, and
/// create the photos row — all in a single transaction.  Deletes the
/// unencrypted temp files on success.
#[allow(clippy::too_many_arguments)]
async fn encrypt_and_store_copy(
    state: &AppState,
    auth: &AuthUser,
    original: &Photo,
    key: [u8; 32],
    new_id: &str,
    copy_filename: &str,
    copy_abs: &std::path::Path,
    thumb_abs: &std::path::Path,
    thumb_rel_opt: Option<&str>,
    storage_root: &std::path::Path,
    created_at: &str,
    new_w: i64,
    new_h: i64,
    new_duration: Option<f64>,
    size_bytes: i64,
) -> Result<(String, Option<String>, Option<String>, Option<String>), AppError> {
    // Read the rendered file data
    let file_data = tokio::fs::read(copy_abs).await.map_err(|e| {
        AppError::Internal(format!("Failed to read rendered copy: {e}"))
    })?;

    // Read thumbnail data (if generated)
    let thumb_data = if thumb_rel_opt.is_some() {
        tokio::fs::read(thumb_abs).await.ok()
    } else {
        None
    };

    // Encrypt and store thumbnail blob
    let mut thumb_blob_id_str = String::new();
    let thumb_insert_params = if let Some(ref tb) = thumb_data {
        // Read actual thumbnail dimensions instead of hardcoding 256×256
        let (thumb_w, thumb_h) = imagesize::blob_size(tb)
            .map(|s| (s.width as i64, s.height as i64))
            .unwrap_or((512, 512));
        tracing::info!(
            "[editing/save_copy] Encrypting thumbnail: {}×{}, {} bytes",
            thumb_w, thumb_h, tb.len(),
        );
        let thumb_payload = serde_json::json!({
            "v": 1,
            "photo_blob_id": "",
            "width": thumb_w,
            "height": thumb_h,
            "data": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, tb,
            ),
        });
        let thumb_json = serde_json::to_vec(&thumb_payload).map_err(|e| {
            AppError::Internal(format!("Thumb JSON failed: {e}"))
        })?;

        let enc_thumb = {
            let kc = key;
            let jc = thumb_json;
            tokio::task::spawn_blocking(move || crypto::encrypt(&kc, &jc))
                .await
                .map_err(|e| AppError::Internal(format!("Thumb encrypt panicked: {e}")))?
                .map_err(|e| AppError::Internal(format!("Thumb encrypt failed: {e}")))?
        };

        let enc_thumb_hash = hex::encode(Sha256::digest(&enc_thumb));
        let tid = Uuid::new_v4().to_string();
        let ttype = if original.media_type == "video" {
            "video_thumbnail"
        } else {
            "thumbnail"
        };
        let tpath = storage::write_blob(storage_root, &auth.user_id, &tid, &enc_thumb)
            .await
            .map_err(|e| AppError::Internal(format!("Write thumb blob: {e}")))?;
        let tnow = Utc::now().to_rfc3339();
        thumb_blob_id_str = tid.clone();
        Some((
            tid,
            ttype.to_string(),
            enc_thumb.len() as i64,
            enc_thumb_hash,
            tnow,
            tpath,
        ))
    } else {
        None
    };

    // Classify blob type
    let blob_type = if original.mime_type == "image/gif" {
        "gif"
    } else if original.mime_type.starts_with("video/") {
        "video"
    } else if original.mime_type.starts_with("audio/") {
        "audio"
    } else {
        "photo"
    };

    // Build photo payload (same format as encrypt_one_photo)
    let photo_payload = serde_json::json!({
        "v": 1,
        "filename": copy_filename,
        "taken_at": original.taken_at.as_deref().unwrap_or(created_at),
        "mime_type": original.mime_type,
        "media_type": original.media_type,
        "width": new_w,
        "height": new_h,
        "duration": new_duration.or(original.duration_secs),
        "latitude": original.latitude,
        "longitude": original.longitude,
        "album_ids": [],
        "thumbnail_blob_id": thumb_blob_id_str,
        "data": base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD, &file_data,
        ),
    });
    let photo_json = serde_json::to_vec(&photo_payload).map_err(|e| {
        AppError::Internal(format!("Photo JSON failed: {e}"))
    })?;

    // Encrypt the photo payload
    let enc_photo = {
        let kc = key;
        let jc = photo_json;
        tokio::task::spawn_blocking(move || crypto::encrypt(&kc, &jc))
            .await
            .map_err(|e| AppError::Internal(format!("Photo encrypt panicked: {e}")))?
            .map_err(|e| AppError::Internal(format!("Photo encrypt failed: {e}")))?
    };

    let enc_photo_hash = hex::encode(Sha256::digest(&enc_photo));
    let blob_id = Uuid::new_v4().to_string();
    let blob_storage_path =
        storage::write_blob(storage_root, &auth.user_id, &blob_id, &enc_photo)
            .await
            .map_err(|e| AppError::Internal(format!("Write photo blob: {e}")))?;

    let now = Utc::now().to_rfc3339();

    // Atomic transaction: INSERT blob rows + INSERT photos row
    let mut tx = state.pool.begin().await.map_err(|e| {
        AppError::Internal(format!("Begin tx: {e}"))
    })?;

    if let Some((ref tid, ref ttype, tsize, ref thash, ref ttime, ref tpath)) = thumb_insert_params
    {
        sqlx::query(
            "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, \
             upload_time, storage_path, content_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(tid)
        .bind(&auth.user_id)
        .bind(ttype)
        .bind(tsize)
        .bind(thash)
        .bind(ttime)
        .bind(tpath)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Internal(format!("Insert thumb blob: {e}")))?;
    }

    sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, client_hash, \
         upload_time, storage_path, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&blob_id)
    .bind(&auth.user_id)
    .bind(blob_type)
    .bind(enc_photo.len() as i64)
    .bind(&enc_photo_hash)
    .bind(&now)
    .bind(&blob_storage_path)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(format!("Insert photo blob: {e}")))?;

    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
         size_bytes, width, height, duration_secs, taken_at, latitude, longitude, \
         thumb_path, created_at, encrypted_blob_id, encrypted_thumb_blob_id, \
         is_favorite, crop_metadata, camera_model, photo_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, NULL, ?, NULL)",
    )
    .bind(new_id)
    .bind(&auth.user_id)
    .bind(copy_filename)
    .bind("") // no unencrypted file_path
    .bind(&original.mime_type)
    .bind(&original.media_type)
    .bind(size_bytes)
    .bind(new_w)
    .bind(new_h)
    .bind(new_duration.or(original.duration_secs))
    .bind(&original.taken_at)
    .bind(original.latitude)
    .bind(original.longitude)
    .bind(Option::<&str>::None) // no unencrypted thumb_path
    .bind(created_at)
    .bind(&blob_id)
    .bind(if thumb_blob_id_str.is_empty() {
        None::<&str>
    } else {
        Some(thumb_blob_id_str.as_str())
    })
    .bind(&original.camera_model)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(format!("Insert photo row: {e}")))?;

    tx.commit().await.map_err(|e| {
        AppError::Internal(format!("Commit tx: {e}"))
    })?;

    // Delete the unencrypted temp files — they must not persist on disk
    let _ = tokio::fs::remove_file(copy_abs).await;
    if thumb_rel_opt.is_some() {
        let _ = tokio::fs::remove_file(thumb_abs).await;
    }

    Ok((
        String::new(),
        Option::<String>::None,
        Some(blob_id),
        if thumb_blob_id_str.is_empty() {
            None
        } else {
            Some(thumb_blob_id_str)
        },
    ))
}
