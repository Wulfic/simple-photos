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
        }
    }

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
            w = pw;
            h = ph;
        }
    }

    (w, h, cam, lat, lon, taken)
}

/// Use ffprobe to get the display dimensions of a video, accounting for SAR/DAR.
async fn probe_video_display_dimensions(path: &std::path::Path) -> Option<(i64, i64)> {
    let output = tokio::process::Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height,sample_aspect_ratio",
            "-of", "csv=p=0:s=,",
        ])
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .ok()?;

    let s = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = s.trim().split(',').collect();
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
    let display_w = (coded_w * sar).round() as i64;
    let display_h = coded_h as i64;

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
