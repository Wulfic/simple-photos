//! Shared per-file registration for native media.
//!
//! The manual `/scan` endpoint ([`crate::photos::scan::scan_and_register`]) and
//! the background / bulk-import autoscan ([`crate::backup::autoscan`]) both walk
//! the storage tree and register unregistered native files. They used to carry
//! two hand-copied versions of the per-file body, which drifted apart into real
//! bugs: `/scan` never excluded gallery-hidden originals (a secure-gallery leak)
//! and the autoscan never extracted embedded motion-photo videos.
//!
//! This module is the single source of truth. A caller does the cheap directory
//! walk to produce [`NativeCandidate`]s, then registers each one — ideally with
//! bounded concurrency (see [`crate::photos::scan::scan_parallelism`]) — via
//! [`register_native_file`].

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::photos::metadata::{
    apply_aspect_subtype_fallback_with, extract_media_metadata_async, extract_xmp_subtype_async,
    PanoSensitivity,
};
use crate::photos::motion::extract_and_store_motion_video;
use crate::photos::thumbnail::generate_thumbnail_file;
use crate::photos::utils::{compute_photo_hash_streaming, normalize_iso_timestamp, utc_now_iso};

/// An unregistered native media file discovered on disk during the walk phase.
///
/// The walk resolves everything cheap (paths, MIME, size, mtime) so the
/// expensive per-file work (EXIF, hashing, thumbnail) can be fanned out.
pub(crate) struct NativeCandidate {
    /// Absolute path on disk.
    pub abs_path: PathBuf,
    /// Storage-root-relative path with forward slashes (the DB `file_path`).
    pub rel_path: String,
    /// File name (the DB `filename`).
    pub name: String,
    /// MIME type derived from the extension.
    pub mime: String,
    /// `"photo" | "video" | "audio" | "gif"`.
    pub media_type: &'static str,
    /// File size in bytes.
    pub size: i64,
    /// File mtime as a normalized ISO string, used as the `taken_at` fallback
    /// when EXIF carries no capture date.
    pub modified: Option<String>,
}

/// Read-only context shared across every file in one registration pass.
pub(crate) struct RegisterContext {
    /// Owner the new rows are assigned to.
    pub user_id: String,
    /// Panorama-detection sensitivity, resolved once per pass (item #7).
    pub pano_sensitivity: PanoSensitivity,
    /// Content hashes of gallery-hidden originals (secure gallery). Any file
    /// whose content hash matches one of these is skipped so a secure-gallery
    /// original can never be re-imported into the normal gallery.
    pub gallery_hashes: Arc<HashSet<String>>,
}

/// Register a single native media file: extract metadata + subtype, hash it,
/// exclude gallery-hidden originals, `INSERT OR IGNORE`, extract an embedded
/// motion video (motion photos), and generate a thumbnail.
///
/// Returns `true` iff a new row was inserted (so callers can count). Original
/// files are never modified or deleted. Safe to run concurrently: the
/// `INSERT OR IGNORE` collapses races between overlapping passes.
pub(crate) async fn register_native_file(
    pool: &SqlitePool,
    storage_root: &Path,
    cand: &NativeCandidate,
    ctx: &RegisterContext,
) -> bool {
    let photo_id = Uuid::new_v4().to_string();
    let now = utc_now_iso();
    // GIFs keep an animated GIF thumbnail; everything else gets a JPEG.
    let thumb_ext = if cand.mime == "image/gif" {
        "gif"
    } else {
        "jpg"
    };
    let thumb_rel = format!(".thumbnails/{photo_id}.thumb.{thumb_ext}");

    // Header-only metadata extraction (dimensions, camera, GPS, capture date).
    let (img_w, img_h, cam_model, exif_lat, exif_lon, exif_taken, exif_taken_offset) =
        extract_media_metadata_async(cand.abs_path.clone()).await;

    // XMP subtype (motion / panorama / 360 / HDR / burst) — photos only. Scanning
    // a video's bytes for XMP is meaningless and the aspect fallback would
    // mis-flag wide videos as panoramas.
    let subtype_info = if cand.media_type == "photo" {
        let mut info = extract_xmp_subtype_async(cand.abs_path.clone()).await;
        apply_aspect_subtype_fallback_with(&mut info, img_w, img_h, ctx.pano_sensitivity);
        info
    } else {
        Default::default()
    };

    let final_taken_at = exif_taken
        .map(|t| normalize_iso_timestamp(&t))
        .or_else(|| cand.modified.clone());

    // Content hash for dedup (streaming — never loads the whole file into RAM).
    let photo_hash = compute_photo_hash_streaming(&cand.abs_path).await;

    // A file whose content matches a gallery-hidden original belongs to a secure
    // gallery item and must stay hidden — never register it in the main gallery.
    if let Some(ref h) = photo_hash {
        if ctx.gallery_hashes.contains(h) {
            tracing::info!(
                file = %cand.rel_path,
                hash = %h,
                "Skipping — content hash matches gallery-hidden original"
            );
            return false;
        }
    }

    let insert_result = sqlx::query(
        "INSERT OR IGNORE INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
         size_bytes, width, height, taken_at, latitude, longitude, camera_model, thumb_path, \
         created_at, photo_hash, photo_subtype, burst_id, taken_at_offset) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&photo_id)
    .bind(&ctx.user_id)
    .bind(&cand.name)
    .bind(&cand.rel_path)
    .bind(&cand.mime)
    .bind(cand.media_type)
    .bind(cand.size)
    .bind(img_w)
    .bind(img_h)
    .bind(&final_taken_at)
    .bind(exif_lat)
    .bind(exif_lon)
    .bind(&cam_model)
    .bind(&thumb_rel)
    .bind(&now)
    .bind(&photo_hash)
    .bind(&subtype_info.photo_subtype)
    .bind(&subtype_info.burst_id)
    .bind(&exif_taken_offset)
    .execute(pool)
    .await;

    match insert_result {
        Ok(result) if result.rows_affected() == 0 => {
            tracing::debug!(file = %cand.rel_path, "Already registered (concurrent scan), skipping");
            return false;
        }
        Err(e) => {
            tracing::error!(file = %cand.rel_path, error = %e, "Failed to register photo");
            return false;
        }
        Ok(_) => {}
    }

    // Motion photo: store the embedded video trailer. Stills only, so the full
    // read here stays bounded; real videos never reach this branch.
    if subtype_info.photo_subtype.as_deref() == Some("motion") {
        let file_bytes = tokio::fs::read(&cand.abs_path).await.unwrap_or_default();
        if !file_bytes.is_empty() {
            extract_and_store_motion_video(
                pool,
                storage_root,
                &ctx.user_id,
                &photo_id,
                &file_bytes,
                subtype_info.motion_video_offset,
            )
            .await;
        }
    }

    if let Some(ref st) = subtype_info.photo_subtype {
        tracing::info!(
            file = %cand.rel_path,
            photo_subtype = %st,
            burst_id = ?subtype_info.burst_id,
            "Registered special photo subtype"
        );
    }

    // Generate the thumbnail last so a failure here still leaves a usable row.
    let thumb_abs = storage_root.join(&thumb_rel);
    if !generate_thumbnail_file(&cand.abs_path, &thumb_abs, &cand.mime, None).await {
        tracing::warn!(file = %cand.rel_path, "Failed to generate thumbnail");
    }

    true
}
