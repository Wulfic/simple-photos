//! EXIF and media metadata extraction (file + in-memory byte entry points).
//! Split out from the former monolithic `metadata.rs`; behavior unchanged.

use super::*;

/// Convert an EXIF `DateTimeOriginal` string (`"YYYY:MM:DD HH:MM:SS"`) plus an
/// optional `OffsetTimeOriginal` (`"+09:00"`, `"-08:00"`, `"Z"`, …) into a
/// canonical UTC ISO-8601 instant, returning `(utc_iso, offset)`.
///
/// EXIF `DateTimeOriginal` is *local wall-clock time* with no embedded zone.
/// The previous implementation appended `"Z"` unconditionally, which silently
/// treated that local time as UTC — every photo captured outside UTC then
/// landed in the wrong timeline slot by its zone offset (the core Google
/// Takeout "wrong dates" bug). When the file also carries the zone via
/// `OffsetTimeOriginal`, we now apply it to recover the true instant and
/// report the normalised offset (e.g. `"+09:00"`) so callers can persist the
/// original zone. When no offset is present the instant is genuinely
/// unknowable, so we fall back to the legacy assume-UTC behaviour and report
/// `None` for the offset.
///
/// The input is derived from attacker-controlled file bytes, so slicing must
/// be char-boundary safe: we require a pure-ASCII string of at least 19 bytes
/// (where 1 byte == 1 char) and use checked `get(..)` slicing. The previous
/// implementation sliced by byte index after a *byte*-length check, which
/// panicked when a crafted EXIF field placed a multi-byte UTF-8 char on a
/// slice boundary.
fn exif_datetime_to_iso(dt_str: &str, offset: Option<&str>) -> Option<(String, Option<String>)> {
    if !dt_str.is_ascii() || dt_str.len() < 19 {
        return None;
    }
    let year = dt_str.get(0..4)?;
    let month = dt_str.get(5..7)?;
    let day = dt_str.get(8..10)?;
    let time = dt_str.get(11..19)?;
    let naive = format!("{year}-{month}-{day}T{time}");

    // When the file records the capture zone, apply it to get the true UTC
    // instant. Build an RFC-3339 string and let chrono do the arithmetic.
    if let Some(off) = offset.and_then(normalize_exif_offset) {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&format!("{naive}{off}")) {
            let utc = dt
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            return Some((utc, Some(off)));
        }
    }

    // No usable offset — assume UTC (legacy behaviour), zone unknown.
    Some((format!("{naive}Z"), None))
}

/// Normalise a raw EXIF offset field (`OffsetTimeOriginal` / `OffsetTime`) into
/// a strict `"+HH:MM"` / `"-HH:MM"` form suitable for RFC-3339. Returns `None`
/// for the "undefined" placeholder EXIF writers emit (blank, `":  "`, etc.) or
/// anything out of range. Accepts `"Z"`/`"z"` and both `±HH:MM` and `±HHMM`.
fn normalize_exif_offset(raw: &str) -> Option<String> {
    let s = raw.trim().trim_matches('"').trim();
    if s.is_empty() {
        return None;
    }
    if s.eq_ignore_ascii_case("z") {
        return Some("+00:00".to_string());
    }
    let bytes = s.as_bytes();
    let sign = match bytes.first()? {
        b'+' => '+',
        b'-' => '-',
        _ => return None,
    };
    // Pull the digits out of the remainder so both "+09:00" and "+0900" work.
    let digits: String = s[1..].chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 4 {
        return None;
    }
    let hh: i32 = digits.get(0..2)?.parse().ok()?;
    let mm: i32 = digits.get(2..4)?.parse().ok()?;
    // Real-world zones span -12:00..=+14:00; reject garbage that would still
    // parse as RFC-3339 but represent no real offset.
    if hh > 14 || mm > 59 {
        return None;
    }
    Some(format!("{sign}{hh:02}:{mm:02}"))
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
    let mut taken_at_offset: Option<String> = None;

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
                        Some(format!("{mk} {md}"))
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
                        let lat_signed = if lat_ref_str.contains('S') { -lat } else { lat };
                        let lon_signed = if lon_ref_str.contains('W') { -lon } else { lon };
                        // Exactly (0,0) — "null island" — is the classic
                        // no-fix value; treat it as "no location".
                        if lat_signed.abs() > 1e-7 || lon_signed.abs() > 1e-7 {
                            latitude = Some(lat_signed);
                            longitude = Some(lon_signed);
                        }
                    }
                }
            }

            // Date taken (EXIF DateTimeOriginal), zone-corrected via
            // OffsetTimeOriginal (falling back to OffsetTime) when present.
            let offset_str: Option<String> = exif_reader
                .get_field(exif::Tag::OffsetTimeOriginal, exif::In::PRIMARY)
                .or_else(|| exif_reader.get_field(exif::Tag::OffsetTime, exif::In::PRIMARY))
                .map(|f| f.display_value().to_string());
            if let Some(dt_field) =
                exif_reader.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
            {
                let dt_str = dt_field
                    .display_value()
                    .to_string()
                    .trim_matches('"')
                    .to_string();
                // EXIF format: "2024:01:15 14:30:00" → convert to UTC ISO 8601
                if let Some((iso, off)) = exif_datetime_to_iso(&dt_str, offset_str.as_deref()) {
                    taken_at = Some(iso);
                    taken_at_offset = off;
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
                        if (5..=8).contains(&orient) {
                            std::mem::swap(&mut width, &mut height);
                            tracing::info!(
                                "[metadata] Swapped dims for EXIF orientation {}: \
                                 now {}×{} for {}",
                                orient,
                                width,
                                height,
                                file_path.display(),
                            );
                        }
                    }
                }
            }
        }
    }

    tracing::debug!(
        "[metadata] Final metadata for {}: {}×{}, camera={:?}, taken_at={:?} (offset={:?})",
        file_path.display(),
        width,
        height,
        camera_model,
        taken_at,
        taken_at_offset,
    );

    (
        width,
        height,
        camera_model,
        latitude,
        longitude,
        taken_at,
        taken_at_offset,
    )
}

/// Extract metadata from raw bytes (for upload_photo where file is in memory).
pub(crate) fn extract_media_metadata_from_bytes(data: &[u8], filename: &str) -> MediaMetadata {
    let mut width: i64 = 0;
    let mut height: i64 = 0;
    let mut camera_model: Option<String> = None;
    let mut latitude: Option<f64> = None;
    let mut longitude: Option<f64> = None;
    let mut taken_at: Option<String> = None;
    let mut taken_at_offset: Option<String> = None;

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
                    Some(format!("{mk} {md}"))
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
                    let lat_signed = if lat_ref_str.contains('S') { -lat } else { lat };
                    let lon_signed = if lon_ref_str.contains('W') { -lon } else { lon };
                    // Exactly (0,0) — "null island" — is the classic value a
                    // camera writes when it has a GPS chip but no fix.  Treat
                    // it as "no location" rather than placing the photo in
                    // the Gulf of Guinea.
                    if lat_signed.abs() > 1e-7 || lon_signed.abs() > 1e-7 {
                        latitude = Some(lat_signed);
                        longitude = Some(lon_signed);
                    }
                }
            }
        }

        let offset_str: Option<String> = exif_reader
            .get_field(exif::Tag::OffsetTimeOriginal, exif::In::PRIMARY)
            .or_else(|| exif_reader.get_field(exif::Tag::OffsetTime, exif::In::PRIMARY))
            .map(|f| f.display_value().to_string());
        if let Some(dt_field) =
            exif_reader.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        {
            let dt_str = dt_field
                .display_value()
                .to_string()
                .trim_matches('"')
                .to_string();
            if let Some((iso, off)) = exif_datetime_to_iso(&dt_str, offset_str.as_deref()) {
                taken_at = Some(iso);
                taken_at_offset = off;
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
                    if (5..=8).contains(&orient) {
                        std::mem::swap(&mut width, &mut height);
                    }
                }
            }
        }
    }

    let _ = filename; // suppress unused warning
    (
        width,
        height,
        camera_model,
        latitude,
        longitude,
        taken_at,
        taken_at_offset,
    )
}

// ── Async wrappers ──────────────────────────────────────────────────────────

/// Async wrapper around [`extract_media_metadata`] that offloads the blocking
/// file I/O and EXIF parsing to a `spawn_blocking` thread.
pub(crate) async fn extract_media_metadata_async(file_path: std::path::PathBuf) -> MediaMetadata {
    let (mut w, mut h, cam, lat, lon, taken, taken_offset) = tokio::task::spawn_blocking({
        let p = file_path.clone();
        move || extract_media_metadata(&p)
    })
    .await
    .unwrap_or((0, 0, None, None, None, None, None));

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
                file_path.display(),
                w,
                h,
                pw,
                ph,
            );
            w = pw;
            h = ph;
        }
    }

    tracing::info!(
        "[metadata] extract_media_metadata_async result for {}: {}×{}, is_video={}",
        file_path.display(),
        w,
        h,
        is_video,
    );

    (w, h, cam, lat, lon, taken, taken_offset)
}

/// Use ffprobe to get the display dimensions of a video, accounting for
/// SAR/DAR and container-level rotation (portrait phone videos).
async fn probe_video_display_dimensions(path: &std::path::Path) -> Option<(i64, i64)> {
    let mut cmd = tokio::process::Command::new("ffprobe");
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height,sample_aspect_ratio:stream_side_data=rotation:format_tags=rotate",
        "-of",
        "csv=p=0:s=,",
    ])
    .arg(path)
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null());
    let output = crate::process::run_with_timeout(&mut cmd, crate::process::FFPROBE_TIMEOUT)
        .await
        .ok()?;

    let s = String::from_utf8_lossy(&output.stdout);
    tracing::debug!(
        "[metadata] ffprobe raw output for {}: {:?}",
        path.display(),
        s.trim(),
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
            if d > 0.0 {
                n / d
            } else {
                1.0
            }
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
    // `get(3..)` — ffprobe omits absent fields entirely, so a two-field
    // line must not panic the scan task.
    let has_90_270_rotation = parts.get(3..).unwrap_or(&[]).iter().any(|p| {
        let trimmed = p.trim();
        // Match rotation values that indicate portrait: 90, -90, 270, -270
        matches!(trimmed, "90" | "-90" | "270" | "-270")
    });
    if has_90_270_rotation {
        tracing::info!(
            "[metadata] Video has 90/270° rotation, swapping {}×{} → {}×{}",
            display_w,
            display_h,
            display_h,
            display_w,
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
        .unwrap_or((0, 0, None, None, None, None, None))
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

    tracing::info!(
        "[DIM-REPAIR] Checking {} photos for orientation fix",
        rows.len()
    );

    let mut fixed = 0u64;
    for (photo_id, file_path, db_w, db_h) in &rows {
        let abs_path = storage_root.join(file_path);
        if !abs_path.exists() {
            continue;
        }

        let path_clone = abs_path.clone();
        let (new_w, new_h, _, _, _, _, _) = extract_media_metadata_async(path_clone).await;

        if new_w > 0 && new_h > 0 && (new_w != *db_w || new_h != *db_h) {
            if let Err(e) = sqlx::query("UPDATE photos SET width = ?, height = ? WHERE id = ?")
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
                    file_path,
                    db_w,
                    db_h,
                    new_w,
                    new_h
                );
            }
        }
    }

    tracing::info!(
        "[DIM-REPAIR] Complete: fixed {} of {} photos",
        fixed,
        rows.len()
    );

    // Mark as done so this doesn't re-run
    let _ = sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('orientation_dim_fix_v2', 'true') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(pool)
    .await;
}

#[cfg(test)]
mod tests {
    use super::{exif_datetime_to_iso, normalize_exif_offset};

    #[test]
    fn exif_no_offset_assumes_utc_and_reports_no_zone() {
        // Legacy behaviour is preserved when the file records no zone: the
        // local wall-clock is treated as UTC and the offset is unknown.
        let (iso, off) = exif_datetime_to_iso("2024:01:15 14:30:00", None).unwrap();
        assert_eq!(iso, "2024-01-15T14:30:00Z");
        assert_eq!(off, None);
    }

    #[test]
    fn exif_positive_offset_converts_to_utc() {
        // Tokyo capture (UTC+9): 14:30 local == 05:30 UTC.
        let (iso, off) = exif_datetime_to_iso("2024:01:15 14:30:00", Some("+09:00")).unwrap();
        assert_eq!(iso, "2024-01-15T05:30:00Z");
        assert_eq!(off.as_deref(), Some("+09:00"));
    }

    #[test]
    fn exif_negative_offset_converts_to_utc() {
        // Pacific capture (UTC-8): 14:30 local == 22:30 UTC.
        let (iso, off) = exif_datetime_to_iso("2024:01:15 14:30:00", Some("-08:00")).unwrap();
        assert_eq!(iso, "2024-01-15T22:30:00Z");
        assert_eq!(off.as_deref(), Some("-08:00"));
    }

    #[test]
    fn exif_half_hour_offset_converts_to_utc() {
        // India (UTC+5:30): 14:30 local == 09:00 UTC.
        let (iso, off) = exif_datetime_to_iso("2024:01:15 14:30:00", Some("+05:30")).unwrap();
        assert_eq!(iso, "2024-01-15T09:00:00Z");
        assert_eq!(off.as_deref(), Some("+05:30"));
    }

    #[test]
    fn exif_offset_rolls_date_backwards() {
        // 02:00 local at UTC+9 lands on the previous calendar day in UTC.
        let (iso, _) = exif_datetime_to_iso("2024:01:15 02:00:00", Some("+09:00")).unwrap();
        assert_eq!(iso, "2024-01-14T17:00:00Z");
    }

    #[test]
    fn exif_z_offset_is_utc() {
        let (iso, off) = exif_datetime_to_iso("2024:01:15 14:30:00", Some("Z")).unwrap();
        assert_eq!(iso, "2024-01-15T14:30:00Z");
        assert_eq!(off.as_deref(), Some("+00:00"));
    }

    #[test]
    fn exif_colonless_offset_is_accepted() {
        // Some writers emit "+0900" instead of "+09:00".
        let (iso, off) = exif_datetime_to_iso("2024:01:15 14:30:00", Some("+0900")).unwrap();
        assert_eq!(iso, "2024-01-15T05:30:00Z");
        assert_eq!(off.as_deref(), Some("+09:00"));
    }

    #[test]
    fn exif_quoted_offset_from_display_value_is_trimmed() {
        // `Field::display_value()` renders ASCII fields wrapped in quotes.
        let (iso, off) = exif_datetime_to_iso("2024:01:15 14:30:00", Some("\"-05:00\"")).unwrap();
        assert_eq!(iso, "2024-01-15T19:30:00Z");
        assert_eq!(off.as_deref(), Some("-05:00"));
    }

    #[test]
    fn exif_undefined_offset_falls_back_to_utc() {
        // EXIF writers emit a blank/placeholder offset when the zone is unknown;
        // it must not corrupt the instant — fall back to assume-UTC.
        for garbage in ["", "   ", ":  ", "+", "abc", "+9", "+09:99", "+15:00"] {
            let (iso, off) = exif_datetime_to_iso("2024:01:15 14:30:00", Some(garbage)).unwrap();
            assert_eq!(
                iso, "2024-01-15T14:30:00Z",
                "offset {garbage:?} should be ignored"
            );
            assert_eq!(off, None, "offset {garbage:?} should not be stored");
        }
    }

    #[test]
    fn exif_malformed_datetime_returns_none() {
        assert!(exif_datetime_to_iso("2024:01:15", Some("+09:00")).is_none());
        assert!(exif_datetime_to_iso("", None).is_none());
        // Non-ASCII must not panic on slice boundaries.
        assert!(exif_datetime_to_iso("2024:01:15 14:30:0é", None).is_none());
    }

    #[test]
    fn normalize_offset_edge_cases() {
        assert_eq!(normalize_exif_offset("+09:00").as_deref(), Some("+09:00"));
        assert_eq!(normalize_exif_offset("-0800").as_deref(), Some("-08:00"));
        assert_eq!(normalize_exif_offset("Z").as_deref(), Some("+00:00"));
        assert_eq!(normalize_exif_offset("+14:00").as_deref(), Some("+14:00"));
        // Out of range / malformed / undefined → None.
        assert_eq!(normalize_exif_offset("+15:00"), None);
        assert_eq!(normalize_exif_offset("+09:99"), None);
        assert_eq!(normalize_exif_offset(""), None);
        assert_eq!(normalize_exif_offset("09:00"), None);
    }
}
