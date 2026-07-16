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

use super::metadata::{
    extract_media_metadata_async, extract_media_metadata_from_bytes_async, extract_xmp_subtype,
};
use super::thumbnail::generate_thumbnail_file;
use super::utils::{audio_backup_enabled, compute_photo_hash, utc_now_iso};
use chrono::Utc;

/// Read and sanitise a Takeout source-album header (`X-Source-Album` /
/// `X-Source-Album-Title`).
///
/// The value is percent-encoded by the client because HTTP header values are
/// bytes, not text: a Latin-1-unsafe album name ("Trip to 東京") cannot be put in
/// a raw header at all — `fetch` throws on it — and anything above ASCII would
/// otherwise arrive mojibake'd.
///
/// The result is sanitised exactly like a user-typed album name (dangerous
/// codepoints stripped, whitespace collapsed, length capped).
fn source_album_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(name)?.to_str().ok()?;
    let decoded = percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .ok()?;
    sanitize::sanitize_display_name(&decoded, MAX_SOURCE_ALBUM_LEN).ok()
}

/// The Takeout album *folder* an upload declares, or `None`.
///
/// Google's date / container folders are rejected here as well as in the browser
/// (`isNonAlbumFolder` in `web/src/utils/uploadAlbums.ts`): the header is
/// client-supplied, and a buggy or hostile client must not be able to turn
/// "Photos from 2021" — which holds a copy of the user's entire library — into an
/// album.
///
/// Deliberately not applied to the *title*: that rule is about folder names, and
/// an album is free to be titled anything.
fn upload_album_name(headers: &HeaderMap) -> Option<String> {
    let album = source_album_header(headers, "X-Source-Album")?;
    if crate::import::sidecar::is_non_album_folder(&album) {
        tracing::debug!(album = %album, "Ignoring non-album Takeout folder from upload header");
        return None;
    }
    Some(album)
}

/// Album names/titles are capped exactly like a user-created album's name.
const MAX_SOURCE_ALBUM_LEN: usize = 200;

/// Record the Takeout album membership an upload declared, if any, through the
/// same shared writer every other import path uses. A failed membership must
/// never fail the upload itself — the photo is already safely stored, and a
/// re-run of the import (or the album backfill) repairs the membership.
async fn record_upload_album(state: &AppState, user_id: &str, photo_id: &str, headers: &HeaderMap) {
    let Some(album) = upload_album_name(headers) else {
        return;
    };
    let title = source_album_header(headers, "X-Source-Album-Title");
    let now = utc_now_iso();
    let _ = crate::import::takeout::record_source_album(
        &state.pool,
        user_id,
        photo_id,
        &album,
        title.as_deref(),
        &now,
    )
    .await;
}

/// POST /api/photos/upload
/// Upload a photo/video/GIF file from a mobile client.
/// The file body is sent as raw bytes with metadata in custom headers:
///   X-Filename: original filename
///   X-Mime-Type: MIME type (e.g., image/jpeg)
///   X-Source-Album: percent-encoded Takeout album folder name (optional)
///   X-Source-Album-Title: percent-encoded real album title (optional)
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
    // Sanitize the user-supplied filename: strip path separators, traversal sequences,
    // control characters, and bidi overrides before any path operations.
    let filename = sanitize::sanitize_filename(&filename);

    // Reject unsupported file formats — accept native + convertible types.
    // This is expected for formats we deliberately don't support (e.g. SVG,
    // which the `image` crate cannot decode). Log it rather than treating it
    // as noteworthy — the web client already drops these at the boundary, so
    // anything reaching here is a non-web client; a debug line is enough for
    // diagnostics without spamming the default-level log.
    if !is_supported_extension(&filename) && !conversion::is_convertible(&filename) {
        tracing::debug!(
            user_id = %auth.user_id,
            filename = %filename,
            "Rejected upload of unsupported file format"
        );
        return Err(AppError::BadRequest(format!(
            "Unsupported file format: '{}'. Accepted: browser-native formats \
             (JPEG, PNG, GIF, WebP, AVIF, BMP, ICO, MP4, WebM, MP3, FLAC, OGG, WAV) \
             and convertible formats (HEIC, TIFF, MKV, AVI, MOV, WMA, AIFF, M4A, etc.).",
            filename.rsplit('.').next().unwrap_or("unknown")
        )));
    }

    // ── Deferred conversion for bulk import (two-phase, like autoscan) ───
    // The admin Import page sets `X-Defer-Conversion` so importing a folder
    // of HEIC / MKV / etc. doesn't stall the sequential upload loop on a slow
    // per-file FFmpeg run. Converting inline blocks the HTTP response, and one
    // hung/slow file freezes the whole import — exactly the Windows-vs-Ubuntu
    // divergence we traced (the automated autoscan path is two-phase, the
    // manual Import path was inline). Instead, drop the raw original into the
    // storage tree and let the SAME background pass the autoscan uses
    // (`run_conversion_pass`) convert + register + encrypt it. Result:
    // import-all-first, convert-later on every platform and entry point.
    //
    // Guards:
    //   • convertible only — native files upload fast and never block.
    //   • admin only — the conversion pass attributes new photos to the admin
    //     user, so deferring a non-admin upload would misattribute it; they
    //     fall through to the inline path below.
    //   • no metadata OVERRIDES (X-Taken-At / GPS) — the background pass reads
    //     metadata from the file and can't replay sidecar values, so Google
    //     Photos Takeout uploads stay on the inline path. X-File-Modified-At
    //     is preserved by stamping the written file's mtime.
    //   • no X-Source-Album — same reason: album membership is client-derived
    //     (the browser saw the folder structure; the server only gets loose
    //     bytes), and deferring returns before any photo row exists to hang the
    //     membership off. A convertible file in a Takeout album folder therefore
    //     stays inline so its album survives.
    if conversion::conversion_target(&filename).is_some()
        && header_truthy(&headers, "X-Defer-Conversion")
        && headers.get("X-Taken-At").is_none()
        && headers.get("X-Latitude").is_none()
        && headers.get("X-Longitude").is_none()
        && headers.get("X-Source-Album").is_none()
        && crate::setup::admin::require_admin(&state, &auth)
            .await
            .is_ok()
    {
        return defer_convertible_upload(&state, &filename, &headers, body).await;
    }

    // ── Convert non-native formats to browser-native equivalents ────
    // Save original upload bytes so we can extract EXIF metadata from them
    // BEFORE conversion (FFmpeg/ImageMagick strips EXIF from the output).
    let original_upload = if conversion::is_convertible(&filename) {
        Some((body.to_vec(), filename.clone()))
    } else {
        None
    };

    let (body, filename, mime_type) = if let Some(target) = conversion::conversion_target(&filename)
    {
        let tmp_dir = state
            .config
            .storage
            .root
            .join(".tmp")
            .join("sp_upload_conv");
        tokio::fs::create_dir_all(&tmp_dir)
            .await
            .map_err(|e| AppError::Internal(format!("Create conversion temp dir: {e}")))?;

        // Canonicalize the temp dir so the path-injection sanitizer below
        // operates against a fully resolved root (no symlinks, no `..`).
        let canonical_tmp_dir = tokio::fs::canonicalize(&tmp_dir)
            .await
            .map_err(|e| AppError::Internal(format!("Canonicalize tmp dir: {e}")))?;

        let conv_id = Uuid::new_v4();
        // Restrict the input temp-file extension to alphanumeric characters only
        // to prevent path-component injection via the user-supplied filename.
        let input_ext: String = filename
            .rsplit('.')
            .next()
            .map(|e| e.chars().filter(|c| c.is_alphanumeric()).collect())
            .filter(|e: &String| !e.is_empty())
            .unwrap_or_else(|| "bin".to_string());
        let tmp_input = canonical_tmp_dir.join(format!("{conv_id}_in.{input_ext}"));
        let tmp_output = canonical_tmp_dir.join(format!("{}_out.{}", conv_id, target.extension));

        // Defense-in-depth path-injection barrier: even though the only
        // user-derived component is the alphanumeric-filtered extension,
        // verify the constructed temp paths cannot escape the canonicalized
        // temp directory before any filesystem operation touches them.
        if !tmp_input.starts_with(&canonical_tmp_dir) || !tmp_output.starts_with(&canonical_tmp_dir)
        {
            return Err(AppError::BadRequest("invalid upload filename".into()));
        }

        // Write uploaded bytes to temp file for ffmpeg
        tokio::fs::write(&tmp_input, &body)
            .await
            .map_err(|e| AppError::Internal(format!("Write temp input: {e}")))?;

        // Make this upload visible in the global ConversionBanner so the
        // user knows their HEIC/MKV/TIFF is still in flight — without
        // this, transient sub-second conversions never showed a banner.
        // Paired `progress_finish_one()` runs in *both* arms of the
        // match below to keep counters accurate even on failure.
        conversion::progress_add(1);

        // A single interactive upload is a lone, serial transcode — let ffmpeg
        // auto-detect threads (all cores). The bounded per-encode thread cap is
        // only for the bulk ingest pass that runs many encodes in parallel.
        let conv_result = conversion::convert_file(&tmp_input, &tmp_output, &target, None).await;

        // Always clean up input
        let _ = tokio::fs::remove_file(&tmp_input).await;

        match conv_result {
            Ok(()) => {
                let converted_bytes = tokio::fs::read(&tmp_output)
                    .await
                    .map_err(|e| AppError::Internal(format!("Read converted file: {e}")))?;
                let _ = tokio::fs::remove_file(&tmp_output).await;
                conversion::progress_finish_one();

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
                conversion::progress_finish_one();
                return Err(AppError::Internal(format!(
                    "Media conversion failed for '{filename}': {e}"
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

    let mut media_type = crate::media::media_type_from_mime(&mime_type);
    // Content-based GIF rescue (#14): an uploaded GIF renamed to a non-`.gif`
    // name (or sent with a generic MIME) would otherwise be tagged `photo` and
    // never appear in the GIF smart album. The plaintext bytes are already in
    // memory here, so sniff them directly.
    let mut mime_type = mime_type;
    if let Some((m, t)) = crate::media::gif_override(media_type, body.as_ref()) {
        tracing::info!(filename = %filename, "Reclassified upload as GIF from content signature");
        mime_type = m.to_string();
        media_type = t;
    }

    // Honor the `audio_backup_enabled` server toggle.  When audio backup is
    // disabled, the multipart upload endpoint must reject audio outright —
    // anything else (silently dropping the body, registering then deleting,
    // etc.) results in user confusion and orphan blobs.  Returning 403 makes
    // it possible for clients to surface a clear "audio backup is disabled"
    // error to the user.
    if media_type == "audio" && !audio_backup_enabled(&state.pool).await {
        tracing::info!(
            user_id = %auth.user_id,
            filename = %filename,
            "Rejecting audio upload: audio_backup_enabled is false"
        );
        return Err(AppError::Forbidden(
            "Audio backup is disabled by server policy".to_string(),
        ));
    }

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
        // A deduped upload still carries album membership. Takeout ships the SAME
        // bytes in the date folder and in every album folder, so for a Takeout
        // upload this branch is where most memberships arrive: skipping it would
        // silently drop every album member that also lives in a date folder —
        // which, for Takeout, is all of them.
        record_upload_album(&state, &auth.user_id, &eid, &headers).await;
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
        .map_err(|e| AppError::Internal(format!("Failed to create uploads directory: {e}")))?;

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
        final_filename = format!("{stem}-{counter}.{ext}");
        counter += 1;
    }

    // Write file to disk
    let file_path = uploads_dir.join(&final_filename);
    tokio::fs::write(&file_path, &body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write photo file: {e}")))?;

    // Relative path for DB storage
    let rel_path = format!("uploads/{final_filename}");

    // Register in database
    let photo_id = Uuid::new_v4().to_string();
    let now = utc_now_iso();
    // Use .thumb.gif for GIFs to preserve animation in thumbnails
    let thumb_ext = if mime_type == "image/gif" {
        "gif"
    } else {
        "jpg"
    };
    let thumb_rel = format!(".thumbnails/{photo_id}.thumb.{thumb_ext}");

    // Extract metadata — use the file-based extractor which includes ffprobe
    // SAR/DAR correction for videos (imagesize::blob_size returns coded
    // dimensions that ignore non-square pixels, leading to squished display).
    // When the file was converted, also extract from the original upload bytes
    // for EXIF dates/GPS/camera, since conversion strips EXIF from the output.
    // Preserve the ORIGINAL bytes for XMP subtype detection below. Conversion
    // (e.g. HEIC→JPEG) strips GPano/GCamera/hdrgm markers, so reading the
    // converted output would silently lose motion/panorama/360/HDR/burst
    // classification. Mirrors scan.rs / ingest.rs, which always scan the
    // original file prefix.
    let original_xmp_bytes: Option<Vec<u8>> = original_upload
        .as_ref()
        .map(|(orig_bytes, _)| orig_bytes.clone());

    let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken, exif_taken_offset) =
        if let Some((orig_bytes, orig_filename)) = original_upload {
            let (_, _, orig_cam, orig_lat, orig_lon, orig_taken, orig_taken_offset) =
                extract_media_metadata_from_bytes_async(orig_bytes, orig_filename).await;
            let (conv_w, conv_h, conv_cam, conv_lat, conv_lon, conv_taken, conv_taken_offset) =
                extract_media_metadata_async(file_path.clone()).await;
            // Keep taken_at and its zone offset paired: take the offset from
            // whichever source supplied the timestamp we keep, never a mix.
            let (taken, taken_offset) = if orig_taken.is_some() {
                (orig_taken, orig_taken_offset)
            } else {
                (conv_taken, conv_taken_offset)
            };
            (
                conv_w,
                conv_h,
                orig_cam.or(conv_cam),
                orig_lat.or(conv_lat),
                orig_lon.or(conv_lon),
                taken,
                taken_offset,
            )
        } else {
            extract_media_metadata_async(file_path.clone()).await
        };

    // ── XMP subtype detection ───────────────────────────────────────────
    // Detect motion photo, panorama, 360, HDR, or burst subtype from embedded
    // XMP. For converted uploads we MUST scan the original bytes (the converted
    // output has no XMP); for native uploads the on-disk file IS the original.
    let xmp_data: Vec<u8> = match original_xmp_bytes {
        Some(bytes) => bytes,
        None => tokio::fs::read(&file_path).await.unwrap_or_default(),
    };
    let mut subtype_info = extract_xmp_subtype(&xmp_data);

    // ── Aspect-ratio fallback ───────────────────────────────────────────
    // When XMP is missing/stripped (common for scanned/exported panoramas
    // and 360° photos that lost their GPano markers), fall back to the
    // image dimensions so the gallery still routes them to the correct
    // viewer. Sensitivity honours the user's AI toggle (item #7): precise
    // thresholds by default, loose only when AI categorisation is off.
    let pano_sensitivity =
        crate::photos::metadata::pano_sensitivity_for_user(&state.read_pool, &auth.user_id).await;
    crate::photos::metadata::apply_aspect_subtype_fallback_with(
        &mut subtype_info,
        img_w,
        img_h,
        pano_sensitivity,
    );

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

    // ── Optional client-supplied metadata overrides ─────────────────────
    // Sidecar-aware uploaders (e.g. the web Import page processing Google
    // Photos Takeout JSON) can pass X-Taken-At / X-Latitude / X-Longitude
    // headers when sidecars supply data the file's EXIF lacks. EXIF still
    // wins when present so that on-device camera metadata isn't overridden
    // by stale sidecar values.
    //
    // X-File-Modified-At carries the browser File's `lastModified` (epoch
    // milliseconds). It's used ONLY as a fallback when both EXIF and any
    // explicit X-Taken-At sidecar value are missing. This mirrors the
    // autoscan pipeline (which falls back to on-disk mtime), so a manually
    // uploaded EXIF-less photo lands in the same timeline slot as it would
    // if it had been dropped on the import directory and autoscanned —
    // instead of being stamped "now" and floating to the top.
    let header_taken_at: Option<String> = headers
        .get("X-Taken-At")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let header_latitude: Option<f64> = headers
        .get("X-Latitude")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|f| f.is_finite() && (-90.0..=90.0).contains(f) && *f != 0.0);
    let header_longitude: Option<f64> = headers
        .get("X-Longitude")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|f| f.is_finite() && (-180.0..=180.0).contains(f) && *f != 0.0);
    let header_file_modified_at: Option<String> = headers
        .get("X-File-Modified-At")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|ms| *ms > 0)
        .and_then(chrono::DateTime::<Utc>::from_timestamp_millis)
        .map(|dt| dt.to_rfc3339());

    // ── Date-taken priority (offset-aware) ──────────────────────────────
    // Fixes "a lot of media with wrong dates": EXIF `DateTimeOriginal` is local
    // wall-clock with no zone, so an offset-less value is only *assumed* UTC and
    // lands non-UTC photos in the wrong day. Google Takeout's `photoTakenTime`
    // (the `X-Taken-At` sidecar) is a true UTC epoch and must beat that guess.
    // The full priority order lives in `resolve_upload_taken_at` (unit-tested).
    let final_taken_at = crate::photos::utils::resolve_upload_taken_at(
        exif_taken.as_deref(),
        exif_taken_offset.is_some(),
        header_taken_at.as_deref(),
        header_file_modified_at.as_deref(),
        &now,
    );

    // Original capture-zone offset (e.g. "+09:00"). The extractor only sets this
    // alongside an EXIF DateTimeOriginal that had a real offset, and that is the
    // ONLY branch that keeps the EXIF value — so whenever this is `Some` the
    // stored `final_taken_at` genuinely came from that zoned EXIF value. Sidecar
    // epochs (Takeout) and mtimes carry no zone, so it stays `None` for those.
    let final_taken_offset = exif_taken_offset;

    let resolved_lat = exif_lat.or(header_latitude);
    let resolved_lon = exif_lon.or(header_longitude);
    // GPS is meaningless without both coordinates — drop the partial value
    // so callers can't poison the DB by supplying only one.
    let (resolved_lat, resolved_lon) = match (resolved_lat, resolved_lon) {
        (Some(la), Some(lo)) => (Some(la), Some(lo)),
        _ => (None, None),
    };

    // ── Geo scrubbing ───────────────────────────────────────────────────
    // If the user has geo-scrubbing enabled, null out GPS coordinates before
    // storing in the database.
    let (insert_lat, insert_lon) =
        if crate::geo::scrub::is_scrub_enabled(&state.pool, &auth.user_id).await {
            (None, None)
        } else {
            (resolved_lat, resolved_lon)
        };

    // Use INSERT OR IGNORE so a concurrent upload race (two near-simultaneous
    // uploads of identical content from different clients) doesn't surface as
    // a 500 from the (user_id, photo_hash) UNIQUE constraint. If we lose the
    // race, look up the row that won and return that — same semantics as
    // hitting the dedup check above.
    let insert_result = sqlx::query(
        "INSERT OR IGNORE INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
         size_bytes, width, height, taken_at, latitude, longitude, camera_model, \
         thumb_path, created_at, photo_hash, photo_subtype, burst_id, taken_at_offset) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(&final_taken_offset)
    .execute(&state.pool)
    .await?;

    if insert_result.rows_affected() == 0 {
        // Lost the race to another concurrent upload of identical content —
        // remove the orphan file we just wrote and return the winning row.
        let _ = tokio::fs::remove_file(&file_path).await;
        let winner: Option<(String, String, String, i64, Option<String>)> = sqlx::query_as(
            "SELECT id, filename, file_path, size_bytes, photo_hash FROM photos \
             WHERE user_id = ? AND photo_hash = ? LIMIT 1",
        )
        .bind(&auth.user_id)
        .bind(&photo_hash)
        .fetch_optional(&state.read_pool)
        .await?;
        if let Some((eid, efn, efp, esz, ehash)) = winner {
            tracing::info!(
                user_id = %auth.user_id,
                filename = %efn,
                photo_hash = %photo_hash,
                "Concurrent upload race resolved — returning winner's record"
            );
            // We lost the race but still hold this copy's album membership;
            // record it against the winning row (idempotent).
            record_upload_album(&state, &auth.user_id, &eid, &headers).await;
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
        return Err(AppError::Internal(
            "Photo insert ignored but no existing row found".to_string(),
        ));
    }

    // Capture Takeout album membership the browser derived from the picked
    // folder structure (issue: "Local Upload captures no albums at all").
    record_upload_album(&state, &auth.user_id, &photo_id, &headers).await;

    // ── Inline geo & timeline backfill ──────────────────────────────────
    // Set photo_year/photo_month from taken_at timestamp
    let _ =
        crate::geo::processor::set_photo_year_month(&state.pool, &photo_id, &final_taken_at).await;

    // A GPS photo just landed — wake the geo processor so its city/country
    // resolves within moments rather than on the next 5-min poll tick.
    if resolved_lat.is_some() {
        state.geo_trigger.notify_one();
    }

    // ── Extract and store motion video blob ─────────────────────────────
    // If the photo is a motion photo with an embedded MP4 trailer, extract it
    // and store it as a separate blob for efficient serving.
    if subtype_info.photo_subtype.as_deref() == Some("motion") {
        super::motion::extract_and_store_motion_video(
            &state.pool,
            &storage_root,
            &auth.user_id,
            &photo_id,
            &xmp_data,
            subtype_info.motion_video_offset,
        )
        .await;
    }

    // ── Generate thumbnail SYNCHRONOUSLY (parity with autoscan) ─────────
    // Autoscan awaits thumbnail generation BEFORE handing off to encryption,
    // so by the time `auto_migrate_after_scan` runs the cache file at
    // `.thumbnails/{id}.thumb.{ext}` already exists and `build_thumbnail`
    // reuses it. Spawning the thumbnail here instead would race encryption:
    // when encryption wins, it falls back to `generate_thumbnail_for_migration`
    // (basic 512×512 resize, no panorama awareness, no FFmpeg path for
    // video/GIF) — that's the encrypted thumbnail the gallery serves
    // forever. Cost: a few hundred ms added to the upload response. Worth it
    // for correctness.
    {
        let thumb_abs = storage_root.join(&thumb_rel);
        if generate_thumbnail_file(&file_path, &thumb_abs, &mime_type, None).await {
            tracing::debug!("Generated thumbnail for uploaded file");
        } else {
            tracing::warn!("Failed to generate thumbnail for uploaded file");
        }
    }

    // ── Hand off to the unified post-scan pipeline ──────────────────────
    // The autoscan flow registers a row, then runs:
    //   auto_migrate_after_scan (native encrypt) → run_conversion_pass.
    // Manual uploads must trigger the EXACT same sequence so we don't end
    // up with two divergent schemes (one that encrypts on the next 5-min
    // autoscan tick, one that encrypts immediately). Spawning keeps the
    // HTTP response fast — the migrator dedupes via a global lock so
    // bursts of uploads coalesce into a single batch instead of racing.
    {
        let pool_clone = state.pool.clone();
        let root_clone = storage_root.clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            crate::photos::server_migrate::auto_migrate_after_scan(
                pool_clone.clone(),
                root_clone.clone(),
                jwt_secret.clone(),
            )
            .await;
            crate::ingest::run_conversion_pass(pool_clone, root_clone, jwt_secret).await;
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

/// Truthy check for a boolean-ish request header (`1`, `true`, `yes`).
fn header_truthy(headers: &HeaderMap, name: &str) -> bool {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            let s = s.trim().to_ascii_lowercase();
            s == "1" || s == "true" || s == "yes"
        })
        .unwrap_or(false)
}

/// Drop a convertible upload's raw bytes into the storage tree and hand off to
/// the background conversion pass (shared with autoscan) for conversion,
/// registration, and encryption. Returns `202 Accepted` immediately so a bulk
/// import never blocks on FFmpeg. See the caller for the guard conditions.
async fn defer_convertible_upload(
    state: &AppState,
    filename: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Honor the audio-backup toggle up front so a disabled-audio server doesn't
    // leave an orphan raw audio file the conversion pass will refuse to touch.
    if let Some(target) = conversion::conversion_target(filename) {
        if target.category == conversion::MediaCategory::Audio
            && !audio_backup_enabled(&state.pool).await
        {
            return Err(AppError::Forbidden(
                "Audio backup is disabled by server policy".to_string(),
            ));
        }
    }

    let storage_root = (**state.storage_root.load()).clone();
    let uploads_dir = storage_root.join("uploads");
    tokio::fs::create_dir_all(&uploads_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create uploads directory: {e}")))?;

    // Unique on-disk name (different content, same name → keep both). Mirrors
    // the inline path's collision handling.
    let safe_filename = sanitize::sanitize_filename(filename);
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
            .unwrap_or("bin");
        final_filename = format!("{stem}-{counter}.{ext}");
        counter += 1;
    }

    let raw_path = uploads_dir.join(&final_filename);
    tokio::fs::write(&raw_path, &body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write import file: {e}")))?;

    // Preserve the browser File's lastModified so EXIF-less files land in the
    // right timeline slot — the conversion pass falls back to on-disk mtime,
    // mirroring autoscan. Without this the file would be stamped with the
    // write time (≈ now) and float to the top of the timeline.
    if let Some(ms) = headers
        .get("X-File-Modified-At")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|ms| *ms > 0)
    {
        let mtime = std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms as u64);
        let raw_path_clone = raw_path.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(f) = std::fs::OpenOptions::new()
                .write(true)
                .open(&raw_path_clone)
            {
                let _ = f.set_modified(mtime);
            }
        })
        .await;
    }

    // Kick the same two-phase pipeline the autoscan uses. `run_conversion_pass`
    // serializes on a global lock, so a burst of deferred uploads coalesces
    // into batched passes rather than racing — identical to the inline path's
    // post-upload hand-off.
    {
        let pool = state.pool.clone();
        let root = storage_root.clone();
        let jwt_secret = state.config.auth.jwt_secret.clone();
        tokio::spawn(async move {
            crate::ingest::run_conversion_pass(pool, root, jwt_secret).await;
        });
    }

    tracing::info!(
        filename = %final_filename,
        "Queued convertible upload for background conversion (deferred)"
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "status": "queued",
            "filename": final_filename,
            "deferred": true,
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        h
    }

    fn album(value: &str) -> Option<String> {
        upload_album_name(&headers_with("X-Source-Album", value))
    }

    fn title(value: &str) -> Option<String> {
        source_album_header(
            &headers_with("X-Source-Album-Title", value),
            "X-Source-Album-Title",
        )
    }

    #[test]
    fn album_header_is_percent_decoded() {
        assert_eq!(album("Trip%20to%20Rome").as_deref(), Some("Trip to Rome"));
    }

    #[test]
    fn album_header_survives_non_ascii_names() {
        // The whole reason the header is encoded: a raw non-Latin-1 value can't
        // be sent at all, and anything above ASCII would arrive mojibake'd.
        assert_eq!(
            album("%E6%9D%B1%E4%BA%AC%202019").as_deref(),
            Some("東京 2019")
        );
    }

    #[test]
    fn album_header_rejects_googles_non_album_folders() {
        // The browser filters these already, but the header is client-supplied.
        assert_eq!(album("Photos%20from%202021"), None);
        assert_eq!(album("Takeout"), None);
        assert_eq!(album("Google%20Photos"), None);
        // A real album that merely looks similar still passes.
        assert_eq!(
            album("Photos%20from%20Grandma").as_deref(),
            Some("Photos from Grandma")
        );
    }

    #[test]
    fn album_header_is_sanitised() {
        // Bidi override stripped, whitespace collapsed (%E2%80%AE is U+202E).
        assert_eq!(
            album("Trip%E2%80%AE%20%20to%20%20%20Rome").as_deref(),
            Some("Trip to Rome")
        );
        // Capped at the same length as any album name.
        assert_eq!(
            album(&"a".repeat(500)).map(|s| s.chars().count()),
            Some(MAX_SOURCE_ALBUM_LEN)
        );
    }

    /// The non-album folder rule is about FOLDER names. An album is free to be
    /// *titled* "Photos from 2021" — dropping that title would silently fall the
    /// album back to its mangled folder name for no reason.
    #[test]
    fn title_header_is_not_subject_to_the_folder_rules() {
        assert_eq!(
            title("Photos%20from%202021").as_deref(),
            Some("Photos from 2021")
        );
        assert_eq!(title("Takeout").as_deref(), Some("Takeout"));
    }

    #[test]
    fn album_header_absent_or_unusable_yields_none() {
        assert_eq!(
            source_album_header(&HeaderMap::new(), "X-Source-Album"),
            None
        );
        assert_eq!(album("%20%20"), None, "blank after sanitisation");
        assert_eq!(album("%E0%A4%A"), None, "invalid percent-encoding");
    }
}
