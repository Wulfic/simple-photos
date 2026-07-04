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

use crate::import::models::GooglePhotosMetadata;
use crate::import::{google_photos, sidecar};
use crate::photos::metadata::{
    apply_aspect_subtype_fallback_with, extract_media_metadata_async, extract_xmp_subtype_async,
    PanoSensitivity,
};
use crate::photos::motion::extract_and_store_motion_video;
use crate::photos::thumbnail::generate_thumbnail_file;
use crate::photos::utils::{compute_photo_hash_streaming, resolve_taken_at, utc_now_iso};

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
    /// when neither EXIF nor a Google Takeout sidecar carries a capture date.
    pub modified: Option<String>,
    /// Absolute path to this file's Google Takeout JSON sidecar, if the walk
    /// found one (see [`crate::import::sidecar`]). The sidecar is the authoritative
    /// source of `taken_at`/GPS for Takeout exports, which frequently strip both
    /// from the JPEG itself.
    pub sidecar_abs: Option<PathBuf>,
    /// Takeout album name derived from the parent folder, when this file lives in
    /// a genuine Takeout album directory. Recorded in `photo_source_albums` so
    /// clients can rebuild albums deterministically.
    pub album_name: Option<String>,
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

/// Record a Google Takeout album membership (idempotent via the
/// `(photo_id, album_name)` primary key). Shared by the fresh-insert path and the
/// hash-duplicate backfill path so an album captures its members regardless of
/// which physical copy of a photo the walk registers first.
async fn record_source_album(
    pool: &SqlitePool,
    user_id: &str,
    photo_id: &str,
    album: &str,
    now: &str,
) {
    if let Err(e) = sqlx::query(
        "INSERT OR IGNORE INTO photo_source_albums \
         (photo_id, user_id, album_name, source, created_at) \
         VALUES (?, ?, ?, 'google_takeout', ?)",
    )
    .bind(photo_id)
    .bind(user_id)
    .bind(album)
    .bind(now)
    .execute(pool)
    .await
    {
        tracing::warn!(photo_id = %photo_id, album = %album, error = %e, "Failed to record Takeout source album");
    }
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

    // Google Takeout sidecar (when the walk paired one): the exported JPEG often
    // has its capture date and GPS stripped, so the sidecar is the ONLY place the
    // true values survive. Parse it up front so `photoTakenTime` can beat the file
    // mtime — which for an unzipped Takeout is the extraction date, not capture.
    let mut sidecar_taken: Option<String> = None;
    let mut sidecar_lat: Option<f64> = None;
    let mut sidecar_lon: Option<f64> = None;
    let mut sidecar_meta: Option<GooglePhotosMetadata> = None;
    if let Some(ref sc_path) = cand.sidecar_abs {
        match tokio::fs::read(sc_path).await {
            Ok(bytes) => match google_photos::parse_sidecar(&bytes) {
                Ok(meta) if sidecar::is_photo_sidecar(&meta) => {
                    let rec =
                        google_photos::normalise(&meta, String::new(), String::new(), None, None);
                    sidecar_taken = rec.taken_at;
                    sidecar_lat = rec.latitude;
                    sidecar_lon = rec.longitude;
                    sidecar_meta = Some(meta);
                }
                Ok(_) => tracing::debug!(
                    file = %cand.rel_path,
                    "Paired .json is not a photo sidecar; ignoring"
                ),
                Err(e) => tracing::warn!(
                    file = %cand.rel_path, error = %e,
                    "Failed to parse Takeout sidecar"
                ),
            },
            Err(e) => tracing::warn!(
                file = %cand.rel_path, sidecar = ?sc_path, error = %e,
                "Failed to read Takeout sidecar"
            ),
        }
    }

    // Capture date priority: zoned EXIF > sidecar epoch > assume-UTC EXIF > mtime.
    let final_taken_at = resolve_taken_at(
        exif_taken.as_deref(),
        exif_taken_offset.is_some(),
        sidecar_taken.as_deref(),
        cand.modified.as_deref(),
    );
    // GPS: prefer embedded EXIF; fall back to the sidecar (Google frequently
    // strips GPS from the file and keeps it only in the JSON).
    let final_lat = exif_lat.or(sidecar_lat);
    let final_lon = exif_lon.or(sidecar_lon);

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
    .bind(final_lat)
    .bind(final_lon)
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
            // Hash-duplicate of an already-registered photo. Google Takeout stores
            // the SAME image bytes in both its "Photos from YYYY" date folder and
            // every album folder the photo belongs to, so the album copies collide
            // on the (user_id, photo_hash) unique index and land here. We must NOT
            // just skip: if this duplicate lives in an album, its membership has to
            // be recorded against the photo that already exists — otherwise an album
            // silently loses every member that also lives in a date folder (which,
            // for Takeout, is all of them). No new photo row is created, so we still
            // return false (callers count new registrations only).
            if let (Some(album), Some(h)) = (cand.album_name.as_deref(), photo_hash.as_ref()) {
                let existing_id: Option<String> = sqlx::query_scalar(
                    "SELECT id FROM photos WHERE user_id = ? AND photo_hash = ? LIMIT 1",
                )
                .bind(&ctx.user_id)
                .bind(h)
                .fetch_optional(pool)
                .await
                .unwrap_or(None);
                match existing_id {
                    Some(existing_id) => {
                        record_source_album(pool, &ctx.user_id, &existing_id, album, &now).await;
                        tracing::debug!(file = %cand.rel_path, album = %album, "Duplicate copy — backfilled album membership onto existing photo");
                    }
                    None => {
                        tracing::warn!(file = %cand.rel_path, "Duplicate on insert but existing photo not found by hash — album membership not recorded")
                    }
                }
            } else {
                tracing::debug!(file = %cand.rel_path, "Already registered (concurrent scan or duplicate), skipping");
            }
            return false;
        }
        Err(e) => {
            tracing::error!(file = %cand.rel_path, error = %e, "Failed to register photo");
            return false;
        }
        Ok(_) => {}
    }

    // Record Takeout album membership captured from the folder (idempotent via
    // the (photo_id, album_name) PK). Only set when the walk decided this file
    // lives in a genuine Takeout album directory, so a normal user folder never
    // becomes an album.
    if let Some(ref album) = cand.album_name {
        record_source_album(pool, &ctx.user_id, &photo_id, album, &now).await;
    }

    // Persist the parsed sidecar (title/description/geo/views) for the info panel.
    // storage_path stays NULL — we don't copy the raw JSON blob on the scan path;
    // the parsed columns carry everything the clients read.
    if let Some(meta) = sidecar_meta {
        let meta_id = Uuid::new_v4().to_string();
        let rec = google_photos::normalise(
            &meta,
            meta_id,
            ctx.user_id.clone(),
            Some(photo_id.clone()),
            None,
        );
        if let Err(e) = sqlx::query(
            "INSERT INTO photo_metadata \
             (id, user_id, photo_id, blob_id, source, title, description, taken_at, \
              created_at_src, latitude, longitude, altitude, image_views, original_url, \
              storage_path, imported_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&rec.id)
        .bind(&rec.user_id)
        .bind(&rec.photo_id)
        .bind(&rec.blob_id)
        .bind(&rec.source)
        .bind(&rec.title)
        .bind(&rec.description)
        .bind(&rec.taken_at)
        .bind(&rec.created_at_src)
        .bind(rec.latitude)
        .bind(rec.longitude)
        .bind(rec.altitude)
        .bind(rec.image_views)
        .bind(&rec.original_url)
        .bind(&rec.storage_path)
        .bind(&rec.imported_at)
        .execute(pool)
        .await
        {
            tracing::warn!(file = %cand.rel_path, error = %e, "Failed to store Takeout photo metadata");
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// End-to-end proof that a paired Google Takeout sidecar drives the DB row —
    /// the exact behaviour the filesystem-scan import was missing: the sidecar's
    /// `photoTakenTime` beats the file mtime, its `geoData` fills the GPS the
    /// exported JPEG had stripped, the album folder is recorded, and a
    /// `photo_metadata` row is written. Thumbnail/EXIF extraction on the dummy
    /// bytes just no-ops (non-fatal), so the assertions isolate the sidecar path.
    #[tokio::test]
    async fn register_applies_takeout_sidecar_date_gps_and_album() {
        // ── A temp Takeout album folder on disk ──
        let root = std::env::temp_dir().join(format!("sp-reg-test-{}", uuid::Uuid::new_v4()));
        let album_dir = root.join("Trip to Rome");
        tokio::fs::create_dir_all(&album_dir).await.unwrap();
        let media = album_dir.join("IMG_1.jpg");
        tokio::fs::write(&media, b"dummy-bytes-just-need-something-to-hash")
            .await
            .unwrap();
        // Sidecar: photoTakenTime = 2017-05-16T19:37:54Z, GPS in Rome.
        let sidecar = album_dir.join("IMG_1.jpg.supplemental-metadata.json");
        tokio::fs::write(
            &sidecar,
            br#"{
                "title":"IMG_1.jpg",
                "photoTakenTime":{"timestamp":"1494963474"},
                "geoData":{"latitude":41.9028,"longitude":12.4964,"altitude":0.0},
                "googlePhotosOrigin":{"mobileUpload":{}}
            }"#,
        )
        .await
        .unwrap();

        // ── In-memory DB with the real migrations (FKs off: we insert a bare
        //    photo row without the full users/blobs graph). ──
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        // ── The candidate exactly as the walker would hand it over ──
        let cand = NativeCandidate {
            abs_path: media.clone(),
            rel_path: "Trip to Rome/IMG_1.jpg".to_string(),
            name: "IMG_1.jpg".to_string(),
            mime: "image/jpeg".to_string(),
            media_type: "photo",
            size: 39,
            // File mtime — the WRONG (extraction-day) date the sidecar overrides.
            modified: Some("2026-07-04T00:00:00.000Z".to_string()),
            sidecar_abs: Some(sidecar.clone()),
            album_name: Some("Trip to Rome".to_string()),
        };
        let ctx = RegisterContext {
            user_id: "user-1".to_string(),
            pano_sensitivity: crate::photos::metadata::PanoSensitivity::Strict,
            gallery_hashes: Arc::new(std::collections::HashSet::new()),
        };

        assert!(
            register_native_file(&pool, &root, &cand, &ctx).await,
            "a new photo row must be inserted"
        );

        // taken_at came from the sidecar epoch, NOT the file mtime.
        let (taken_at, lat, lon): (Option<String>, Option<f64>, Option<f64>) = sqlx::query_as(
            "SELECT taken_at, latitude, longitude FROM photos WHERE user_id = 'user-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            taken_at.as_deref(),
            Some("2017-05-16T19:37:54.000Z"),
            "sidecar photoTakenTime must beat the file mtime"
        );
        assert_eq!(lat, Some(41.9028), "GPS latitude comes from the sidecar");
        assert_eq!(lon, Some(12.4964), "GPS longitude comes from the sidecar");

        // Album membership recorded from the folder.
        let (album,): (String,) =
            sqlx::query_as("SELECT album_name FROM photo_source_albums WHERE user_id = 'user-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(album, "Trip to Rome");

        // Parsed sidecar metadata persisted for the info panel.
        let (meta_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM photo_metadata WHERE user_id = 'user-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            meta_count, 1,
            "a parsed sidecar metadata row must be stored"
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// A plain library (no sidecars) must be untouched: no album invented, mtime
    /// kept as the capture date. Guards against turning every user folder into an
    /// album or regressing non-Takeout imports.
    #[tokio::test]
    async fn register_without_sidecar_keeps_mtime_and_records_no_album() {
        let root = std::env::temp_dir().join(format!("sp-reg-test-{}", uuid::Uuid::new_v4()));
        let dir = root.join("Vacation Photos");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let media = dir.join("plain.jpg");
        tokio::fs::write(&media, b"another-dummy-blob")
            .await
            .unwrap();

        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let cand = NativeCandidate {
            abs_path: media.clone(),
            rel_path: "Vacation Photos/plain.jpg".to_string(),
            name: "plain.jpg".to_string(),
            mime: "image/jpeg".to_string(),
            media_type: "photo",
            size: 18,
            modified: Some("2020-01-02T03:04:05.000Z".to_string()),
            sidecar_abs: None,
            album_name: None,
        };
        let ctx = RegisterContext {
            user_id: "user-1".to_string(),
            pano_sensitivity: crate::photos::metadata::PanoSensitivity::Strict,
            gallery_hashes: Arc::new(std::collections::HashSet::new()),
        };

        assert!(register_native_file(&pool, &root, &cand, &ctx).await);

        let (taken_at,): (Option<String>,) =
            sqlx::query_as("SELECT taken_at FROM photos WHERE user_id = 'user-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(taken_at.as_deref(), Some("2020-01-02T03:04:05.000Z"));

        let (album_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM photo_source_albums WHERE user_id = 'user-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(album_count, 0, "no sidecars → no album invented");

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// THE Takeout album bug: Google stores identical bytes in both the
    /// "Photos from YYYY" date folder and every album folder. The date-folder copy
    /// registers first; the album copy then collides on the (user_id, photo_hash)
    /// unique index and is a dedup no-op. It must STILL record the album membership
    /// against the already-existing photo — otherwise albums come up empty.
    #[tokio::test]
    async fn duplicate_album_copy_backfills_membership_onto_existing_photo() {
        let root = std::env::temp_dir().join(format!("sp-reg-test-{}", uuid::Uuid::new_v4()));
        let year_dir = root.join("Photos from 2020");
        let album_dir = root.join("Cats");
        tokio::fs::create_dir_all(&year_dir).await.unwrap();
        tokio::fs::create_dir_all(&album_dir).await.unwrap();

        // Same bytes in both locations → same content hash → dedup collision.
        let bytes = b"identical-cat-photo-bytes-shared-across-both-folders";
        let year_media = year_dir.join("cat.jpg");
        let album_media = album_dir.join("cat.jpg");
        tokio::fs::write(&year_media, bytes).await.unwrap();
        tokio::fs::write(&album_media, bytes).await.unwrap();

        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let ctx = RegisterContext {
            user_id: "user-1".to_string(),
            pano_sensitivity: crate::photos::metadata::PanoSensitivity::Strict,
            gallery_hashes: Arc::new(std::collections::HashSet::new()),
        };

        // 1) Date-folder copy registers first — no album.
        let year_cand = NativeCandidate {
            abs_path: year_media.clone(),
            rel_path: "Photos from 2020/cat.jpg".to_string(),
            name: "cat.jpg".to_string(),
            mime: "image/jpeg".to_string(),
            media_type: "photo",
            size: bytes.len() as i64,
            modified: Some("2020-06-01T00:00:00.000Z".to_string()),
            sidecar_abs: None,
            album_name: None,
        };
        assert!(
            register_native_file(&pool, &root, &year_cand, &ctx).await,
            "first (date-folder) copy inserts a new photo"
        );

        // 2) Album-folder copy — identical bytes, carries the album name.
        let album_cand = NativeCandidate {
            abs_path: album_media.clone(),
            rel_path: "Cats/cat.jpg".to_string(),
            name: "cat.jpg".to_string(),
            mime: "image/jpeg".to_string(),
            media_type: "photo",
            size: bytes.len() as i64,
            modified: Some("2020-06-01T00:00:00.000Z".to_string()),
            sidecar_abs: None,
            album_name: Some("Cats".to_string()),
        };
        assert!(
            !register_native_file(&pool, &root, &album_cand, &ctx).await,
            "duplicate copy must NOT create a second photo row"
        );

        // Exactly one photo, and the album membership was backfilled onto it.
        let (photo_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM photos WHERE user_id = 'user-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(photo_count, 1, "dedup keeps a single photo row");

        let (album, count): (String, i64) = sqlx::query_as(
            "SELECT album_name, COUNT(*) FROM photo_source_albums WHERE user_id = 'user-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "album membership must be recorded despite dedup");
        assert_eq!(album, "Cats");

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
