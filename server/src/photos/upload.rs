//! POST /api/photos/upload — mobile client photo upload handler.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::conversion;
use crate::error::AppError;
use crate::media::{is_supported_extension, mime_from_extension};
use crate::sanitize;
use crate::state::AppState;

use super::metadata::{extract_media_metadata_async, extract_media_metadata_from_bytes_async, extract_xmp_subtype, extract_motion_video};
use super::thumbnail::generate_thumbnail_file;
use super::utils::{compute_photo_hash, normalize_iso_timestamp, utc_now_iso};
use chrono::Utc;

/// POST /api/photos/upload
/// Upload a photo/video/GIF file from a mobile client.
/// The file body is sent as raw bytes with metadata in custom headers:
///   X-Filename: original filename
///   X-Mime-Type: MIME type (e.g., image/jpeg)
///
/// The server stores the file in the storage root and registers it as a photo.
pub async fn upload_photo(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Reject early if storage backend is unreachable (network drive disconnected)
    if !state.is_storage_available() {
        return Err(AppError::StorageUnavailable);
    }

    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}.jpg", Uuid::new_v4()));

    // Reject unsupported file formats — accept native + convertible types
    if !is_supported_extension(&filename) && !conversion::is_convertible(&filename) {
        return Err(AppError::BadRequest(format!(
            "Unsupported file format: '{}'. Accepted: browser-native formats \
             (JPEG, PNG, GIF, WebP, AVIF, BMP, ICO, MP4, WebM, MP3, FLAC, OGG, WAV) \
             and convertible formats (HEIC, TIFF, RAW, MKV, AVI, MOV, WMA, AIFF, M4A, etc.).",
            filename.rsplit('.').next().unwrap_or("unknown")
        )));
    }

    // ── Convert non-native formats to browser-native equivalents ────
    // Save original upload bytes so we can extract EXIF metadata from them
    // BEFORE conversion (FFmpeg/ImageMagick strips EXIF from the output).
    let original_upload = if conversion::is_convertible(&filename) {
        Some((body.to_vec(), filename.clone()))
    } else {
        None
    };

    let (body, filename, mime_type) = if let Some(target) = conversion::conversion_target(&filename) {
        let tmp_dir = state.config.storage.root.join(".tmp").join("sp_upload_conv");
        let conv_id = Uuid::new_v4();
        let tmp_input = tmp_dir.join(format!("{}_in.{}", conv_id,
            filename.rsplit('.').next().unwrap_or("bin")));
        let tmp_output = tmp_dir.join(format!("{}_out.{}", conv_id, target.extension));

        tokio::fs::create_dir_all(&tmp_dir)
            .await
            .map_err(|e| AppError::Internal(format!("Create conversion temp dir: {}", e)))?;

        // Write uploaded bytes to temp file for ffmpeg
        tokio::fs::write(&tmp_input, &body)
            .await
            .map_err(|e| AppError::Internal(format!("Write temp input: {}", e)))?;

        let conv_result = conversion::convert_file(&tmp_input, &tmp_output, &target).await;

        // Always clean up input
        let _ = tokio::fs::remove_file(&tmp_input).await;

        match conv_result {
            Ok(()) => {
                let converted_bytes = tokio::fs::read(&tmp_output)
                    .await
                    .map_err(|e| AppError::Internal(format!("Read converted file: {}", e)))?;
                let _ = tokio::fs::remove_file(&tmp_output).await;

                // Build new filename with converted extension
                let stem = std::path::Path::new(&filename)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("converted");
                let new_filename = format!("{}.{}", stem, target.extension);
                let new_mime = target.mime_type.to_string();

                tracing::info!(
                    original = %filename,
                    converted = %new_filename,
                    "Converted upload to browser-native format"
                );

                (Bytes::from(converted_bytes), new_filename, new_mime)
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp_output).await;
                return Err(AppError::Internal(format!(
                    "Media conversion failed for '{}': {}", filename, e
                )));
            }
        }
    } else {
        let mime = headers
            .get("X-Mime-Type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| mime_from_extension(&filename).to_string());
        (body, filename, mime)
    };

    let mime_type = mime_type;

    let media_type = if mime_type.starts_with("video/") {
        "video"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else if mime_type == "image/gif" {
        "gif"
    } else {
        "photo"
    };

    let size_bytes = body.len() as i64;

    // Sanitize filename — strip path separators, traversal, and dangerous chars
    let safe_filename = sanitize::sanitize_filename(&filename);

    // ── Content hash for cross-platform alignment ───────────────────────
    let photo_hash = compute_photo_hash(&body);

    // ── Content-aware dedup (hash-based) ────────────────────────────────
    // If a photo with the identical content hash already exists for this
    // user, return it immediately — no duplicate stored.
    let existing: Option<(String, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, filename, file_path, size_bytes, photo_hash FROM photos \
         WHERE user_id = ? AND photo_hash = ? LIMIT 1",
    )
    .bind(&auth.user_id)
    .bind(&photo_hash)
    .fetch_optional(&state.read_pool)
    .await?;

    if let Some((eid, efn, efp, esz, ehash)) = existing {
        tracing::info!(
            user_id = %auth.user_id,
            filename = %efn,
            photo_hash = %photo_hash,
            "Duplicate upload detected (hash match) — returning existing record"
        );
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "photo_id": eid,
                "filename": efn,
                "file_path": efp,
                "size_bytes": esz,
                "photo_hash": ehash,
            })),
        ));
    }

    // Ensure unique filename if it already exists on disk (different content)
    let storage_root = (**state.storage_root.load()).clone();
    let uploads_dir = storage_root.join("uploads");
    tokio::fs::create_dir_all(&uploads_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create uploads directory: {}", e)))?;

    let mut final_filename = safe_filename.clone();
    let mut counter = 1u32;
    while tokio::fs::try_exists(uploads_dir.join(&final_filename))
        .await
        .unwrap_or(false)
    {
        let stem = std::path::Path::new(&safe_filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let ext = std::path::Path::new(&safe_filename)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("jpg");
        final_filename = format!("{}-{}.{}", stem, counter, ext);
        counter += 1;
    }

    // Write file to disk
    let file_path = uploads_dir.join(&final_filename);
    tokio::fs::write(&file_path, &body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write photo file: {}", e)))?;

    // Relative path for DB storage
    let rel_path = format!("uploads/{}", final_filename);

    // Register in database
    let photo_id = Uuid::new_v4().to_string();
    let now = utc_now_iso();
    // Use .thumb.gif for GIFs to preserve animation in thumbnails
    let thumb_ext = if mime_type == "image/gif" {
        "gif"
    } else {
        "jpg"
    };
    let thumb_rel = format!(".thumbnails/{}.thumb.{}", photo_id, thumb_ext);

    // Extract metadata — use the file-based extractor which includes ffprobe
    // SAR/DAR correction for videos (imagesize::blob_size returns coded
    // dimensions that ignore non-square pixels, leading to squished display).
    // When the file was converted, also extract from the original upload bytes
    // for EXIF dates/GPS/camera, since conversion strips EXIF from the output.
    let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken) =
        if let Some((orig_bytes, orig_filename)) = original_upload {
            let (_, _, orig_cam, orig_lat, orig_lon, orig_taken) =
                extract_media_metadata_from_bytes_async(orig_bytes, orig_filename).await;
            let (conv_w, conv_h, conv_cam, conv_lat, conv_lon, conv_taken) =
                extract_media_metadata_async(file_path.clone()).await;
            (
                conv_w,
                conv_h,
                orig_cam.or(conv_cam),
                orig_lat.or(conv_lat),
                orig_lon.or(conv_lon),
                orig_taken.or(conv_taken),
            )
        } else {
            extract_media_metadata_async(file_path.clone()).await
        };

    // ── XMP subtype detection ───────────────────────────────────────────
    // Read original file bytes to detect motion photo, panorama, 360, HDR,
    // or burst subtype from embedded XMP metadata.
    let xmp_data = tokio::fs::read(&file_path).await.unwrap_or_default();
    let subtype_info = extract_xmp_subtype(&xmp_data);

    match &subtype_info.photo_subtype {
        Some(subtype) => {
            tracing::info!(
                user_id = %auth.user_id,
                filename = %final_filename,
                photo_subtype = %subtype,
                burst_id = ?subtype_info.burst_id,
                motion_video_offset = ?subtype_info.motion_video_offset,
                "Upload: special photo subtype detected"
            );
        }
        None => {
            tracing::debug!(
                user_id = %auth.user_id,
                filename = %final_filename,
                "Upload: no XMP subtype detected (standard photo)"
            );
        }
    }

    let final_taken_at = exif_taken
        .map(|t| normalize_iso_timestamp(&t))
        .unwrap_or_else(|| now.clone());

    // ── Geo scrubbing ───────────────────────────────────────────────────
    // If the user has geo-scrubbing enabled, null out GPS coordinates before
    // storing in the database.
    let (insert_lat, insert_lon) = if crate::geo::scrub::is_scrub_enabled(&state.pool, &auth.user_id).await {
        (None, None)
    } else {
        (exif_lat, exif_lon)
    };

    sqlx::query(
        "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
         size_bytes, width, height, taken_at, latitude, longitude, camera_model, \
         thumb_path, created_at, photo_hash, photo_subtype, burst_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&photo_id)
    .bind(&auth.user_id)
    .bind(&final_filename)
    .bind(&rel_path)
    .bind(&mime_type)
    .bind(media_type)
    .bind(size_bytes)
    .bind(img_w)
    .bind(img_h)
    .bind(&final_taken_at)
    .bind(insert_lat)
    .bind(insert_lon)
    .bind(&cam_model)
    .bind(&thumb_rel)
    .bind(&now)
    .bind(&photo_hash)
    .bind(&subtype_info.photo_subtype)
    .bind(&subtype_info.burst_id)
    .execute(&state.pool)
    .await?;

    // ── Inline geo & timeline backfill ──────────────────────────────────
    // Set photo_year/photo_month from taken_at timestamp
    let _ = crate::geo::processor::set_photo_year_month(&state.pool, &photo_id, &final_taken_at).await;

    // ── Extract and store motion video blob ─────────────────────────────
    // If the photo is a motion photo with an embedded MP4 trailer, extract it
    // and store it as a separate blob for efficient serving.
    if subtype_info.photo_subtype.as_deref() == Some("motion") {
        if let Some(offset) = subtype_info.motion_video_offset {
            if let Some(video_bytes) = extract_motion_video(&xmp_data, offset) {
                let blob_id = Uuid::new_v4().to_string();
                let blob_storage_dir = storage_root.join("blobs");
                let _ = tokio::fs::create_dir_all(&blob_storage_dir).await;
                let blob_rel = format!("blobs/{}.mp4", blob_id);
                let blob_abs = storage_root.join(&blob_rel);

                if tokio::fs::write(&blob_abs, &video_bytes).await.is_ok() {
                    let blob_size = video_bytes.len() as i64;
                    let blob_now = Utc::now().to_rfc3339();

                    let insert_ok = sqlx::query(
                        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
                         VALUES (?, ?, 'motion_video', ?, ?, ?)",
                    )
                    .bind(&blob_id)
                    .bind(&auth.user_id)
                    .bind(blob_size)
                    .bind(&blob_now)
                    .bind(&blob_rel)
                    .execute(&state.pool)
                    .await;

                    if insert_ok.is_ok() {
                        let _ = sqlx::query(
                            "UPDATE photos SET motion_video_blob_id = ? WHERE id = ?",
                        )
                        .bind(&blob_id)
                        .bind(&photo_id)
                        .execute(&state.pool)
                        .await;

                        tracing::info!(
                            photo_id = %photo_id,
                            blob_id = %blob_id,
                            size = blob_size,
                            "Extracted and stored motion video blob"
                        );
                    } else {
                        let _ = tokio::fs::remove_file(&blob_abs).await;
                    }
                }
            }
        }
    }

    // Generate thumbnail immediately so it's available for the first gallery load.
    // Runs in the background — the upload response isn't delayed if this is slow.
    {
        let thumb_abs = storage_root.join(&thumb_rel);
        let file_path_clone = file_path.clone();
        let mime_clone = mime_type.clone();
        tokio::spawn(async move {
            if generate_thumbnail_file(&file_path_clone, &thumb_abs, &mime_clone, None).await {
                tracing::debug!("Generated thumbnail for uploaded file");
            } else {
                tracing::warn!("Failed to generate thumbnail for uploaded file");
            }
        });
    }

    tracing::info!(
        user_id = %auth.user_id,
        filename = %final_filename,
        size = size_bytes,
        photo_hash = %photo_hash,
        "Uploaded photo via mobile client"
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "photo_id": photo_id,
            "filename": final_filename,
            "file_path": rel_path,
            "size_bytes": size_bytes,
            "photo_hash": photo_hash,
        })),
    ))
}
