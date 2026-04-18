//! EXIF and media metadata extraction.
//!
//! Provides two entry points:
//! - [`extract_media_metadata`] — reads from a file path (used during scan).
//! - [`extract_media_metadata_from_bytes`] — reads from in-memory bytes
//!   (used during upload).
//!
//! Both extract: image dimensions (via `imagesize`), camera make/model, GPS
//! coordinates, and `DateTimeOriginal` (via the `exif` crate).

/// Metadata tuple returned by both extraction functions.
pub(crate) type MediaMetadata = (
    i64,
    i64,
    Option<String>,
    Option<f64>,
    Option<f64>,
    Option<String>,
);

/// Extended subtype information extracted from XMP metadata.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubtypeInfo {
    /// Photo subtype: "motion", "panorama", "equirectangular", "hdr", "burst"
    pub photo_subtype: Option<String>,
    /// Burst group identifier (shared across shots in a burst)
    pub burst_id: Option<String>,
    /// Byte offset from end-of-file to the start of the embedded MP4
    /// (motion photos only — used to extract the video trailer)
    pub motion_video_offset: Option<u64>,
}

/// Extract image dimensions, camera model, and GPS coordinates from a file.
/// Returns (width, height, camera_model, latitude, longitude, taken_at).
///
/// **Blocking:** Uses `std::fs::File::open` and CPU-bound EXIF parsing.
/// Callers on the tokio runtime should use [`extract_media_metadata_async`]
/// instead, which wraps this in `spawn_blocking`.
pub(crate) fn extract_media_metadata(file_path: &std::path::Path) -> MediaMetadata {
    let mut width: i64 = 0;
    let mut height: i64 = 0;
    let mut camera_model: Option<String> = None;
    let mut latitude: Option<f64> = None;
    let mut longitude: Option<f64> = None;
    let mut taken_at: Option<String> = None;

    // Try to get dimensions using imagesize (fast, header-only read)
    if let Ok(size) = imagesize::size(file_path) {
        width = size.width as i64;
        height = size.height as i64;
    }

    // Try to read EXIF data for camera model, GPS, and date
    if let Ok(file) = std::fs::File::open(file_path) {
        let mut buf_reader = std::io::BufReader::new(&file);
        if let Ok(exif_reader) = exif::Reader::new().read_from_container(&mut buf_reader) {
            // Camera make + model
            let make = exif_reader
                .get_field(exif::Tag::Make, exif::In::PRIMARY)
                .map(|f| f.display_value().to_string().trim().to_string());
            let model = exif_reader
                .get_field(exif::Tag::Model, exif::In::PRIMARY)
                .map(|f| f.display_value().to_string().trim().to_string());
            camera_model = match (make, model) {
                (Some(mk), Some(md)) => {
                    // Remove surrounding quotes from EXIF strings
                    let mk = mk.trim_matches('"').trim().to_string();
                    let md = md.trim_matches('"').trim().to_string();
                    if md.starts_with(&mk) {
                        Some(md)
                    } else {
                        Some(format!("{} {}", mk, md))
                    }
                }
                (None, Some(md)) => Some(md.trim_matches('"').trim().to_string()),
                (Some(mk), None) => Some(mk.trim_matches('"').trim().to_string()),
                _ => None,
            };

            // GPS coordinates
            if let (Some(lat_field), Some(lat_ref), Some(lon_field), Some(lon_ref)) = (
                exif_reader.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY),
                exif_reader.get_field(exif::Tag::GPSLatitudeRef, exif::In::PRIMARY),
                exif_reader.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY),
                exif_reader.get_field(exif::Tag::GPSLongitudeRef, exif::In::PRIMARY),
            ) {
                if let (exif::Value::Rational(ref lat_vals), exif::Value::Rational(ref lon_vals)) =
                    (&lat_field.value, &lon_field.value)
                {
                    if lat_vals.len() >= 3 && lon_vals.len() >= 3 {
                        let lat = lat_vals[0].to_f64()
                            + lat_vals[1].to_f64() / 60.0
                            + lat_vals[2].to_f64() / 3600.0;
                        let lon = lon_vals[0].to_f64()
                            + lon_vals[1].to_f64() / 60.0
                            + lon_vals[2].to_f64() / 3600.0;
                        let lat_ref_str = lat_ref.display_value().to_string();
                        let lon_ref_str = lon_ref.display_value().to_string();
                        latitude = Some(if lat_ref_str.contains('S') { -lat } else { lat });
                        longitude = Some(if lon_ref_str.contains('W') { -lon } else { lon });
                    }
                }
            }

            // Date taken (EXIF DateTimeOriginal)
            if let Some(dt_field) =
                exif_reader.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
            {
                let dt_str = dt_field
                    .display_value()
                    .to_string()
                    .trim_matches('"')
                    .to_string();
                // EXIF format: "2024:01:15 14:30:00" → convert to ISO 8601
                if dt_str.len() >= 19 {
                    let iso = format!(
                        "{}-{}-{}T{}Z",
                        &dt_str[0..4],
                        &dt_str[5..7],
                        &dt_str[8..10],
                        &dt_str[11..19]
                    );
                    taken_at = Some(iso);
                }
            }

            // If imagesize failed but EXIF has dimensions, use those
            if width == 0 || height == 0 {
                if let Some(w_field) =
                    exif_reader.get_field(exif::Tag::PixelXDimension, exif::In::PRIMARY)
                {
                    if let Some(w) = w_field.value.get_uint(0) {
                        width = w as i64;
                    }
                }
                if let Some(h_field) =
                    exif_reader.get_field(exif::Tag::PixelYDimension, exif::In::PRIMARY)
                {
                    if let Some(h) = h_field.value.get_uint(0) {
                        height = h as i64;
                    }
                }
            }

            // EXIF Orientation values 5–8 indicate the image is rotated 90°
            // or 270°, so the displayed width/height are swapped relative to
            // the raw pixel dimensions reported by imagesize.
            if width > 0 && height > 0 {
                if let Some(orient_field) =
                    exif_reader.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                {
                    if let Some(orient) = orient_field.value.get_uint(0) {
                        tracing::debug!(
                            "[metadata] EXIF orientation={} for {}, dims_before_swap={}×{}",
                            orient,
                            file_path.display(),
                            width,
                            height,
                        );
                        if orient >= 5 && orient <= 8 {
                            std::mem::swap(&mut width, &mut height);
                            tracing::info!(
                                "[metadata] Swapped dims for EXIF orientation {}: \
                                 now {}×{} for {}",
                                orient, width, height,
                                file_path.display(),
                            );
                        }
                    }
                }
            }
        }
    }

    tracing::debug!(
        "[metadata] Final metadata for {}: {}×{}, camera={:?}, taken_at={:?}",
        file_path.display(), width, height, camera_model, taken_at,
    );

    (width, height, camera_model, latitude, longitude, taken_at)
}

/// Extract metadata from raw bytes (for upload_photo where file is in memory).
pub(crate) fn extract_media_metadata_from_bytes(
    data: &[u8],
    filename: &str,
) -> (
    i64,
    i64,
    Option<String>,
    Option<f64>,
    Option<f64>,
    Option<String>,
) {
    let mut width: i64 = 0;
    let mut height: i64 = 0;
    let mut camera_model: Option<String> = None;
    let mut latitude: Option<f64> = None;
    let mut longitude: Option<f64> = None;
    let mut taken_at: Option<String> = None;

    // Get dimensions from bytes
    if let Ok(size) = imagesize::blob_size(data) {
        width = size.width as i64;
        height = size.height as i64;
    }

    // EXIF from bytes
    let mut cursor = std::io::Cursor::new(data);
    if let Ok(exif_reader) = exif::Reader::new().read_from_container(&mut cursor) {
        let make = exif_reader
            .get_field(exif::Tag::Make, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string().trim().to_string());
        let model = exif_reader
            .get_field(exif::Tag::Model, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string().trim().to_string());
        camera_model = match (make, model) {
            (Some(mk), Some(md)) => {
                let mk = mk.trim_matches('"').trim().to_string();
                let md = md.trim_matches('"').trim().to_string();
                if md.starts_with(&mk) {
                    Some(md)
                } else {
                    Some(format!("{} {}", mk, md))
                }
            }
            (None, Some(md)) => Some(md.trim_matches('"').trim().to_string()),
            (Some(mk), None) => Some(mk.trim_matches('"').trim().to_string()),
            _ => None,
        };

        if let (Some(lat_field), Some(lat_ref), Some(lon_field), Some(lon_ref)) = (
            exif_reader.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY),
            exif_reader.get_field(exif::Tag::GPSLatitudeRef, exif::In::PRIMARY),
            exif_reader.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY),
            exif_reader.get_field(exif::Tag::GPSLongitudeRef, exif::In::PRIMARY),
        ) {
            if let (exif::Value::Rational(ref lat_vals), exif::Value::Rational(ref lon_vals)) =
                (&lat_field.value, &lon_field.value)
            {
                if lat_vals.len() >= 3 && lon_vals.len() >= 3 {
                    let lat = lat_vals[0].to_f64()
                        + lat_vals[1].to_f64() / 60.0
                        + lat_vals[2].to_f64() / 3600.0;
                    let lon = lon_vals[0].to_f64()
                        + lon_vals[1].to_f64() / 60.0
                        + lon_vals[2].to_f64() / 3600.0;
                    let lat_ref_str = lat_ref.display_value().to_string();
                    let lon_ref_str = lon_ref.display_value().to_string();
                    latitude = Some(if lat_ref_str.contains('S') { -lat } else { lat });
                    longitude = Some(if lon_ref_str.contains('W') { -lon } else { lon });
                }
            }
        }

        if let Some(dt_field) =
            exif_reader.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        {
            let dt_str = dt_field
                .display_value()
                .to_string()
                .trim_matches('"')
                .to_string();
            if dt_str.len() >= 19 {
                let iso = format!(
                    "{}-{}-{}T{}Z",
                    &dt_str[0..4],
                    &dt_str[5..7],
                    &dt_str[8..10],
                    &dt_str[11..19]
                );
                taken_at = Some(iso);
            }
        }

        if width == 0 || height == 0 {
            if let Some(w_field) =
                exif_reader.get_field(exif::Tag::PixelXDimension, exif::In::PRIMARY)
            {
                if let Some(w) = w_field.value.get_uint(0) {
                    width = w as i64;
                }
            }
            if let Some(h_field) =
                exif_reader.get_field(exif::Tag::PixelYDimension, exif::In::PRIMARY)
            {
                if let Some(h) = h_field.value.get_uint(0) {
                    height = h as i64;
                }
            }
        }

        // EXIF Orientation values 5–8 indicate 90°/270° rotation — swap
        if width > 0 && height > 0 {
            if let Some(orient_field) =
                exif_reader.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            {
                if let Some(orient) = orient_field.value.get_uint(0) {
                    if orient >= 5 && orient <= 8 {
                        std::mem::swap(&mut width, &mut height);
                    }
                }
            }
        }
    }

    let _ = filename; // suppress unused warning
    (width, height, camera_model, latitude, longitude, taken_at)
}

// ── Async wrappers ──────────────────────────────────────────────────────────

/// Async wrapper around [`extract_media_metadata`] that offloads the blocking
/// file I/O and EXIF parsing to a `spawn_blocking` thread.
pub(crate) async fn extract_media_metadata_async(file_path: std::path::PathBuf) -> MediaMetadata {
    let (mut w, mut h, cam, lat, lon, taken) =
        tokio::task::spawn_blocking({
            let p = file_path.clone();
            move || extract_media_metadata(&p)
        })
        .await
        .unwrap_or_else(|_| (0, 0, None, None, None, None));

    // For video files, `imagesize` returns coded pixel dimensions which
    // ignore SAR/DAR.  Use ffprobe to get display dimensions so the gallery
    // calculates aspect ratios correctly (avoids squished thumbnails).
    let is_video = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v" | "wmv" | "flv" | "ts" | "mts"
            )
        })
        .unwrap_or(false);

    if is_video {
        if let Some((pw, ph)) = probe_video_display_dimensions(&file_path).await {
            tracing::info!(
                "[metadata] Video ffprobe override for {}: imagesize={}×{} → ffprobe={}×{}",
                file_path.display(), w, h, pw, ph,
            );
            w = pw;
            h = ph;
        }
    }

    tracing::info!(
        "[metadata] extract_media_metadata_async result for {}: {}×{}, is_video={}",
        file_path.display(), w, h, is_video,
    );

    (w, h, cam, lat, lon, taken)
}

/// Use ffprobe to get the display dimensions of a video, accounting for
/// SAR/DAR and container-level rotation (portrait phone videos).
async fn probe_video_display_dimensions(path: &std::path::Path) -> Option<(i64, i64)> {
    let mut cmd = tokio::process::Command::new("ffprobe");
    cmd.args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height,sample_aspect_ratio:stream_side_data=rotation:format_tags=rotate",
            "-of", "csv=p=0:s=,",
        ])
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let output = crate::process::run_with_timeout(
        &mut cmd,
        crate::process::FFPROBE_TIMEOUT,
    )
    .await
    .ok()?;

    let s = String::from_utf8_lossy(&output.stdout);
    tracing::debug!(
        "[metadata] ffprobe raw output for {}: {:?}",
        path.display(), s.trim(),
    );
    // Output may have multiple lines (stream info, side_data, format tags).
    // Collect all parts across lines.
    let all_text = s.trim().replace('\n', ",");
    let parts: Vec<&str> = all_text.split(',').collect();
    if parts.len() < 2 {
        return None;
    }

    let coded_w: f64 = parts[0].trim().parse().ok()?;
    let coded_h: f64 = parts[1].trim().parse().ok()?;

    // Parse SAR (e.g., "40:33", "1:1", or "N/A")
    let sar = if parts.len() >= 3 {
        let sar_str = parts[2].trim();
        if let Some((num, den)) = sar_str.split_once(':') {
            let n: f64 = num.parse().unwrap_or(1.0);
            let d: f64 = den.parse().unwrap_or(1.0);
            if d > 0.0 { n / d } else { 1.0 }
        } else {
            1.0
        }
    } else {
        1.0
    };

    // Display width = coded width × SAR
    let mut display_w = (coded_w * sar).round() as i64;
    let mut display_h = coded_h as i64;

    // Check for rotation in remaining fields: 90 or 270 degrees means portrait.
    // Rotation can appear as side_data rotation or format tag "rotate".
    let has_90_270_rotation = parts[3..].iter().any(|p| {
        let trimmed = p.trim();
        // Match rotation values that indicate portrait: 90, -90, 270, -270
        matches!(trimmed, "90" | "-90" | "270" | "-270")
    });
    if has_90_270_rotation {
        tracing::info!(
            "[metadata] Video has 90/270° rotation, swapping {}×{} → {}×{}",
            display_w, display_h, display_h, display_w,
        );
        std::mem::swap(&mut display_w, &mut display_h);
    }

    if display_w > 0 && display_h > 0 {
        Some((display_w, display_h))
    } else {
        None
    }
}

/// Async wrapper around [`extract_media_metadata_from_bytes`] that offloads
/// the CPU-bound EXIF parsing to a `spawn_blocking` thread.
pub(crate) async fn extract_media_metadata_from_bytes_async(
    data: Vec<u8>,
    filename: String,
) -> MediaMetadata {
    tokio::task::spawn_blocking(move || extract_media_metadata_from_bytes(&data, &filename))
        .await
        .unwrap_or_else(|_| (0, 0, None, None, None, None))
}

/// One-time startup repair: re-read EXIF orientation for every photo that has
/// a file on disk and fix width/height where orientations 5-8 caused the raw
/// pixel dimensions to be stored instead of the display dimensions.
///
/// Guarded by a `server_settings` flag so it runs at most once per database.
pub async fn repair_orientation_dimensions(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
) {
    // Check if already done
    let done: bool = sqlx::query_scalar(
        "SELECT value = 'true' FROM server_settings WHERE key = 'orientation_dim_fix_v2'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(false);

    if done {
        return;
    }

    tracing::info!("[DIM-REPAIR] Starting one-time EXIF orientation dimension repair");

    let rows: Vec<(String, String, i64, i64)> = match sqlx::query_as(
        "SELECT id, file_path, width, height FROM photos \
         WHERE file_path != '' AND width > 0 AND height > 0 \
         AND media_type IN ('photo', 'gif')",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("[DIM-REPAIR] Failed to query photos: {}", e);
            return;
        }
    };

    tracing::info!("[DIM-REPAIR] Checking {} photos for orientation fix", rows.len());

    let mut fixed = 0u64;
    for (photo_id, file_path, db_w, db_h) in &rows {
        let abs_path = storage_root.join(file_path);
        if !abs_path.exists() {
            continue;
        }

        let path_clone = abs_path.clone();
        let (new_w, new_h, _, _, _, _) =
            extract_media_metadata_async(path_clone).await;

        if new_w > 0 && new_h > 0 && (new_w != *db_w || new_h != *db_h) {
            if let Err(e) = sqlx::query(
                "UPDATE photos SET width = ?, height = ? WHERE id = ?",
            )
            .bind(new_w)
            .bind(new_h)
            .bind(photo_id)
            .execute(pool)
            .await
            {
                tracing::warn!("[DIM-REPAIR] Failed to update {}: {}", photo_id, e);
            } else {
                fixed += 1;
                tracing::debug!(
                    "[DIM-REPAIR] Fixed {}: {}x{} -> {}x{}",
                    file_path, db_w, db_h, new_w, new_h
                );
            }
        }
    }

    tracing::info!("[DIM-REPAIR] Complete: fixed {} of {} photos", fixed, rows.len());

    // Mark as done so this doesn't re-run
    let _ = sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('orientation_dim_fix_v2', 'true') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(pool)
    .await;
}

// ── XMP subtype extraction ──────────────────────────────────────────────────

/// Extract photo subtype information from XMP metadata embedded in a JPEG.
///
/// Scans the first 128 KB of the file for an `<x:xmpmeta` block and looks
/// for known XMP properties that indicate motion photos, panoramas, 360°
/// equirectangular projections, HDR Gainmaps, and burst sequences.
pub(crate) fn extract_xmp_subtype(data: &[u8]) -> SubtypeInfo {
    let mut info = SubtypeInfo::default();

    // Read up to 128 KB — XMP is typically within the first few KB
    let search_len = data.len().min(128 * 1024);
    let chunk = &data[..search_len];

    // Try to find XMP as a UTF-8 string in the file header
    let text = match std::str::from_utf8(chunk) {
        Ok(s) => s.to_string(),
        Err(_) => String::from_utf8_lossy(chunk).to_string(),
    };

    // ── Motion Photo detection ──────────────────────────────────────────
    // Old schema: Camera:MicroVideo="1" + Camera:MicroVideoOffset
    // New schema: GCamera:MotionPhoto="1" + GCamera:MotionVideoOffset (or MicroVideoOffset)
    let is_motion = text.contains("MicroVideo=\"1\"")
        || text.contains("MotionPhoto=\"1\"")
        || text.contains("MicroVideo=\"1\"")
        || text.contains("MotionPhoto=\"1\"");

    if is_motion {
        info.photo_subtype = Some("motion".to_string());

        // Extract video offset — try both attribute names
        if let Some(offset) = extract_xmp_int_attr(&text, "MicroVideoOffset")
            .or_else(|| extract_xmp_int_attr(&text, "MotionVideoOffset"))
        {
            info.motion_video_offset = Some(offset);
        }

        tracing::debug!(
            "[xmp] Detected motion photo, video_offset={:?}",
            info.motion_video_offset
        );
        return info;
    }

    // ── Panorama / 360° detection ───────────────────────────────────────
    // XMP: GPano:ProjectionType="equirectangular" or "cylindrical"
    if let Some(proj) = extract_xmp_str_attr(&text, "ProjectionType") {
        let proj_lower = proj.to_ascii_lowercase();
        if proj_lower == "equirectangular" {
            info.photo_subtype = Some("equirectangular".to_string());
            tracing::debug!("[xmp] Detected equirectangular (360°) photo");
            return info;
        } else if proj_lower == "cylindrical" {
            info.photo_subtype = Some("panorama".to_string());
            tracing::debug!("[xmp] Detected cylindrical panorama");
            return info;
        }
    }

    // ── HDR Gainmap detection ───────────────────────────────────────────
    // Ultra HDR: hdrgm:Version present in XMP
    if text.contains("hdrgm:Version") || text.contains("HDRGainMap") {
        info.photo_subtype = Some("hdr".to_string());
        tracing::debug!("[xmp] Detected HDR Gainmap photo");
        return info;
    }

    // ── Burst detection ─────────────────────────────────────────────────
    // Google: GCamera:BurstID or com.google.photos.burst.id
    if let Some(bid) = extract_xmp_str_attr(&text, "BurstID") {
        info.photo_subtype = Some("burst".to_string());
        info.burst_id = Some(bid);
        tracing::debug!("[xmp] Detected burst photo, burst_id={:?}", info.burst_id);
        return info;
    }

    info
}

/// Extract photo subtype from a file on disk (reads first 128 KB).
pub(crate) fn extract_xmp_subtype_from_file(path: &std::path::Path) -> SubtypeInfo {
    match std::fs::read(path) {
        Ok(data) => extract_xmp_subtype(&data),
        Err(e) => {
            tracing::warn!("[xmp] Failed to read file for XMP extraction: {}", e);
            SubtypeInfo::default()
        }
    }
}

/// Async wrapper for file-based XMP extraction.
pub(crate) async fn extract_xmp_subtype_async(path: std::path::PathBuf) -> SubtypeInfo {
    tokio::task::spawn_blocking(move || extract_xmp_subtype_from_file(&path))
        .await
        .unwrap_or_default()
}

/// Extract the embedded MP4 trailer from a motion photo JPEG.
///
/// The MP4 starts at `file_size - offset` bytes from the end of the file.
/// Returns the raw MP4 bytes, or `None` if extraction fails.
pub(crate) fn extract_motion_video(data: &[u8], offset: u64) -> Option<Vec<u8>> {
    let offset = offset as usize;
    if offset == 0 || offset > data.len() {
        tracing::warn!(
            "[xmp] Invalid motion video offset: {} (file size: {})",
            offset,
            data.len()
        );
        return None;
    }

    let start = data.len() - offset;
    let video_bytes = data[start..].to_vec();

    // Sanity check: MP4 files start with a box header — the fourth through
    // eighth bytes should be "ftyp" for a valid ISO base media file.
    if video_bytes.len() >= 8 && &video_bytes[4..8] == b"ftyp" {
        tracing::debug!(
            "[xmp] Extracted motion video: {} bytes (ftyp confirmed)",
            video_bytes.len()
        );
        Some(video_bytes)
    } else if video_bytes.len() >= 8 {
        // Some motion photos have a different box first — still try
        tracing::warn!(
            "[xmp] Motion video does not start with ftyp box (got {:?}), returning anyway",
            &video_bytes[4..8.min(video_bytes.len())]
        );
        Some(video_bytes)
    } else {
        tracing::warn!(
            "[xmp] Motion video too small: {} bytes",
            video_bytes.len()
        );
        None
    }
}

/// Async extraction of motion video from a file on disk.
pub(crate) async fn extract_motion_video_from_file(
    path: &std::path::Path,
    offset: u64,
) -> Option<Vec<u8>> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        match std::fs::read(&path) {
            Ok(data) => extract_motion_video(&data, offset),
            Err(e) => {
                tracing::warn!("[xmp] Failed to read file for motion video: {}", e);
                None
            }
        }
    })
    .await
    .ok()
    .flatten()
}

// ── XMP helpers ─────────────────────────────────────────────────────────────

/// Extract an integer attribute value from XMP text.
/// Looks for patterns like `AttrName="12345"` or `AttrName='12345'`.
fn extract_xmp_int_attr(text: &str, attr_name: &str) -> Option<u64> {
    // Try pattern: AttrName="value"
    let pattern = format!("{}=\"", attr_name);
    if let Some(pos) = text.find(&pattern) {
        let start = pos + pattern.len();
        let rest = &text[start..];
        if let Some(end) = rest.find('"') {
            return rest[..end].trim().parse().ok();
        }
    }
    // Try pattern: AttrName='value'
    let pattern = format!("{}='", attr_name);
    if let Some(pos) = text.find(&pattern) {
        let start = pos + pattern.len();
        let rest = &text[start..];
        if let Some(end) = rest.find('\'') {
            return rest[..end].trim().parse().ok();
        }
    }
    None
}

/// Extract a string attribute value from XMP text.
fn extract_xmp_str_attr(text: &str, attr_name: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr_name);
    if let Some(pos) = text.find(&pattern) {
        let start = pos + pattern.len();
        let rest = &text[start..];
        if let Some(end) = rest.find('"') {
            let val = rest[..end].trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    let pattern = format!("{}='", attr_name);
    if let Some(pos) = text.find(&pattern) {
        let start = pos + pattern.len();
        let rest = &text[start..];
        if let Some(end) = rest.find('\'') {
            let val = rest[..end].trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}
