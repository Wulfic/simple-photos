//! XMP subtype detection, motion-video extraction, subtype backfill, and tests.
//! Split out from the former monolithic `metadata.rs`; behavior unchanged.

use super::*;

// ── XMP subtype extraction ──────────────────────────────────────────────────

/// Extract photo subtype information from XMP metadata embedded in a JPEG.
///
/// Scans the first 128 KB of the file for an `<x:xmpmeta` block and looks
/// for known XMP properties that indicate motion photos, panoramas, 360°
/// equirectangular projections, HDR Gainmaps, and burst sequences.
///
/// `burst_id` is extracted **independently** of the single `photo_subtype`
/// value: real-world Pixel burst frames frequently also carry the Ultra HDR
/// `hdrgm:` marker (and can be motion photos), and dropping the BurstID in
/// those cases silently breaks burst grouping.
///
/// Subtype precedence when several markers are present:
/// motion > panorama/equirectangular > burst > hdr.
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

    // ── Burst ID (orthogonal to subtype) ────────────────────────────────
    // Google: GCamera:BurstID — always captured so grouping survives the
    // subtype precedence rules below.
    if let Some(bid) = extract_xmp_str_attr(&text, "BurstID") {
        tracing::debug!(burst_id = %bid, "[xmp] Burst ID found");
        info.burst_id = Some(bid);
    }

    // ── Motion Photo detection ──────────────────────────────────────────
    // Old schema: Camera:MicroVideo + Camera:MicroVideoOffset
    // New schema: GCamera:MotionPhoto + GCamera:MotionVideoOffset, or the
    // Container:Directory item list (Pixel 2017+) where the video item's
    // Item:Length is the MP4 trailer size from end-of-file.
    //
    // The attribute helpers below tolerate either quote style and any
    // namespace prefix, so a file that emits `gcamera:MotionPhoto='1'`
    // with single quotes is still recognised.  We treat any non-empty,
    // non-"0", non-"false" value as "motion photo present".
    let motion_flag = extract_xmp_str_attr(&text, "MicroVideo")
        .or_else(|| extract_xmp_str_attr(&text, "MotionPhoto"));
    let is_motion = matches!(
        motion_flag.as_deref().map(str::trim),
        Some(v) if !v.is_empty()
            && !v.eq_ignore_ascii_case("0")
            && !v.eq_ignore_ascii_case("false")
    );

    if is_motion {
        info.photo_subtype = Some("motion".to_string());

        // Offset: legacy attributes first, then the modern Container
        // directory schema used by current Pixel firmware (which carries
        // no *Offset attribute at all).
        if let Some(offset) = extract_xmp_int_attr(&text, "MicroVideoOffset")
            .or_else(|| extract_xmp_int_attr(&text, "MotionVideoOffset"))
            .or_else(|| extract_container_video_offset(&text))
        {
            info.motion_video_offset = Some(offset);
        }

        tracing::debug!(
            motion_flag = ?motion_flag,
            video_offset = ?info.motion_video_offset,
            "[xmp] Motion/live photo detected"
        );
        return info;
    }

    // ── Panorama / 360° detection ──────────────────────────────────────
    // XMP: GPano:ProjectionType="equirectangular", "cylindrical", or
    // "fisheye".  Fisheye cannot be sphere-rendered, but it is still a
    // panorama capture — route it to the flat panorama viewer rather
    // than leaving it untagged.
    if let Some(proj) = extract_xmp_str_attr(&text, "ProjectionType") {
        let proj_lower = proj.to_ascii_lowercase();
        if proj_lower == "equirectangular" {
            info.photo_subtype = Some("equirectangular".to_string());
            tracing::debug!(projection = %proj, "[xmp] Panorama detected: equirectangular (360°)");
            return info;
        } else if proj_lower == "cylindrical" || proj_lower == "fisheye" {
            info.photo_subtype = Some("panorama".to_string());
            tracing::debug!(projection = %proj, "[xmp] Panorama detected");
            return info;
        } else {
            tracing::debug!(projection = %proj, "[xmp] GPano:ProjectionType found but unrecognised, ignoring");
        }
    }

    // ── Burst subtype ───────────────────────────────────────────────────
    // Checked BEFORE the HDR marker: Pixel bursts are routinely Ultra HDR,
    // and stacking in the gallery matters more than the HDR badge.
    if info.burst_id.is_some() {
        info.photo_subtype = Some("burst".to_string());
        return info;
    }

    // ── HDR Gainmap detection ───────────────────────────────────────────
    // Ultra HDR: hdrgm:Version present in XMP
    if text.contains("hdrgm:Version") || text.contains("HDRGainMap") {
        info.photo_subtype = Some("hdr".to_string());
        let has_version = text.contains("hdrgm:Version");
        let has_gainmap = text.contains("HDRGainMap");
        tracing::debug!(
            has_hdrgm_version = has_version,
            has_hdr_gainmap = has_gainmap,
            "[xmp] HDR Gainmap (Ultra HDR) photo detected"
        );
        return info;
    }

    info
}

/// Extract the motion-video trailer offset from the modern GCamera
/// `Container:Directory` XMP schema (Pixel "Motion Photo v1", 2017+).
///
/// The directory lists the file's concatenated items in order; the
/// `video/mp4` item's `Item:Length` is the byte length of the MP4 trailer
/// measured from the end of the file.  `Item:Padding`, when present on the
/// same item, sits between the trailer start and the MP4 data and must be
/// included in the offset.
fn extract_container_video_offset(text: &str) -> Option<u64> {
    let vid_pos = text
        .find("Item:Mime=\"video/mp4\"")
        .or_else(|| text.find("Item:Mime='video/mp4'"))?;
    // Bound the attribute search to the enclosing XML element so we don't
    // pick up the primary image item's attributes.
    let elem_start = text[..vid_pos].rfind('<').unwrap_or(0);
    let elem_end = text[vid_pos..]
        .find('>')
        .map(|i| vid_pos + i)
        .unwrap_or(text.len());
    let elem = &text[elem_start..elem_end];
    let len = extract_xmp_int_attr(elem, "Item:Length")?;
    let pad = extract_xmp_int_attr(elem, "Item:Padding").unwrap_or(0);
    Some(len + pad)
}

/// Aspect-ratio fallback for panorama / equirectangular detection.
///
/// Many real-world panoramic images — especially scanned/stitched JPEGs
/// and 360° photos exported by tools that strip XMP — carry **no**
/// `GPano:ProjectionType` marker.  When `extract_xmp_subtype` returns
/// `None`, we still want the gallery to designate them so the proper
/// viewer (panorama scrubber or 360° sphere) is offered.
///
/// Rules (only applied when `info.photo_subtype` is `None`):
/// * `width.max(height) >= 2048` — avoids mis-tagging banners/screenshots.
/// * Horizontal: aspect `>= 2.0` ⇒ `panorama` (cylindrical / wide stitch).
///   Tightened from the previous 1.8 threshold which over-matched cinematic
///   crops and 16:9 landscape shots.
/// * Vertical: `h/w >= 2.5` ⇒ `panorama` (rare Samsung "vertical pano").
/// * Equirectangular (360°): aspect ∈ [1.97, 2.03] **and** `width >= 4000`.
///   Real 360° photo spheres are 4K+ wide; tightening this avoids false
///   positives on ordinary 2:1 wallpapers.
///
/// Width/height of `0` (unknown) are treated as a no-op.
pub(crate) fn apply_aspect_subtype_fallback(info: &mut SubtypeInfo, width: i64, height: i64) {
    if info.photo_subtype.is_some() {
        return;
    }
    if width <= 0 || height <= 0 {
        return;
    }
    // Long edge must be at least 2048 px so we don't tag wide-but-small
    // crops, banners, screenshots, or social-media exports.  Real panoramas
    // are stitched from multiple frames and almost always have a long edge
    // well above 2K pixels.
    if width.max(height) < 2048 {
        return;
    }
    let w = width as f64;
    let h = height as f64;
    let aspect = w / h;
    // Horizontal panorama: w/h ≥ 2.0.  The previous 1.8 threshold was too
    // permissive — common ultra-wide phone landscape shots (~1.8:1) and
    // 18:9 / 19.5:9 cinematic crops were getting tagged as panoramas.
    // Vertical panorama: h/w ≥ 2.5.  True vertical panos (Samsung "vertical
    // pano") are extremely tall; raising this floor stops portrait videos
    // and tall screenshots from being misclassified.
    let is_horizontal_pano = aspect >= 2.0;
    let is_vertical_pano = (1.0 / aspect) >= 2.5;
    if !is_horizontal_pano && !is_vertical_pano {
        return;
    }
    // Equirectangular requires a TIGHT 2:1 ratio AND a high-resolution
    // long edge.  A real 360° photo sphere is typically ≥ 4K wide
    // (Pixel: 7680×3840, RICOH Theta: 5376×2688).  A random 2:1 wallpaper
    // or landscape crop should NOT be flagged as 360°.
    let subtype = if is_horizontal_pano && (1.97..=2.03).contains(&aspect) && width >= 4000 {
        "equirectangular"
    } else {
        "panorama"
    };
    tracing::debug!(
        width,
        height,
        aspect,
        chosen = subtype,
        "[subtype] aspect-ratio fallback assigned panorama subtype"
    );
    info.photo_subtype = Some(subtype.to_string());
}

/// How much of a file the XMP subtype scanner actually needs.  XMP packets
/// sit in the first few KB of JPEG/HEIC files; 256 KB gives generous slack
/// without ever pulling a multi-gigabyte video into memory.
pub(crate) const XMP_SCAN_PREFIX_BYTES: usize = 256 * 1024;

/// Read at most `cap` bytes from the start of a file.  Returns an empty
/// vec on any I/O error (callers treat that as "no metadata found").
pub(crate) async fn read_file_prefix(path: &std::path::Path, cap: usize) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "[xmp] Failed to open file for prefix read");
            return Vec::new();
        }
    };
    let mut buf = Vec::with_capacity(cap.min(64 * 1024));
    let mut handle = file.take(cap as u64);
    if let Err(e) = handle.read_to_end(&mut buf).await {
        tracing::warn!(path = %path.display(), error = %e, "[xmp] Prefix read failed");
        return Vec::new();
    }
    buf
}

/// Extract photo subtype from a file on disk (reads only the XMP prefix,
/// never the whole file).
pub(crate) fn extract_xmp_subtype_from_file(path: &std::path::Path) -> SubtypeInfo {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("[xmp] Failed to read file for XMP extraction: {}", e);
            return SubtypeInfo::default();
        }
    };
    let mut buf = Vec::with_capacity(64 * 1024);
    if let Err(e) = file
        .take(XMP_SCAN_PREFIX_BYTES as u64)
        .read_to_end(&mut buf)
    {
        tracing::warn!("[xmp] Failed to read file for XMP extraction: {}", e);
        return SubtypeInfo::default();
    }
    extract_xmp_subtype(&buf)
}

/// Async wrapper for file-based XMP extraction.
pub(crate) async fn extract_xmp_subtype_async(path: std::path::PathBuf) -> SubtypeInfo {
    tokio::task::spawn_blocking(move || extract_xmp_subtype_from_file(&path))
        .await
        .unwrap_or_default()
}

/// Backfill `photo_subtype` for every photo with `photo_subtype IS NULL`,
/// across **all users**, regardless of which user uploaded them.
///
/// This is the startup-time companion to the per-user logic in
/// [`crate::photos::scan::scan_and_register`]: existing users who imported
/// photos before subtype detection / aspect-ratio fallback was added would
/// otherwise see panoramas keep showing as regular photos until they
/// manually triggered a re-scan.  Running this once on boot upgrades the
/// library in place.
///
/// Bounded by `max_files` to keep startup cost predictable on huge libraries.
/// Photos that fail to update (missing on disk, IO error) are skipped and
/// logged at debug level — nothing aborts the whole pass.
///
/// Returns the number of rows that were updated.
pub async fn backfill_photo_subtypes_all_users(
    pool: &sqlx::SqlitePool,
    storage_root: &std::path::Path,
    max_files: usize,
) -> i64 {
    use futures_util::stream::{self, StreamExt};
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    // XMP subtypes only apply to still photos.  Including videos here used
    // to pull entire multi-GB files into memory for a string scan.
    let rows: Vec<(String, String, i64, i64)> = match sqlx::query_as(
        "SELECT id, file_path, COALESCE(width, 0), COALESCE(height, 0) \
         FROM photos \
         WHERE photo_subtype IS NULL AND file_path != '' \
         AND media_type = 'photo' \
         LIMIT ?",
    )
    .bind(max_files as i64)
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "[subtype-backfill] DB query failed");
            return 0;
        }
    };

    if rows.is_empty() {
        return 0;
    }

    tracing::info!(
        candidates = rows.len(),
        "[subtype-backfill] checking existing photos for missed XMP / aspect subtypes"
    );

    let updated = Arc::new(AtomicI64::new(0));
    let storage_root = storage_root.to_path_buf();
    stream::iter(rows)
        .map(|(pid, fpath, ph_w, ph_h)| {
            let pool = pool.clone();
            let updated = updated.clone();
            let storage_root = storage_root.clone();
            async move {
                let abs = storage_root.join(&fpath);
                if !tokio::fs::try_exists(&abs).await.unwrap_or(false) {
                    return;
                }
                let _ = tokio::spawn(async move {
                    let bytes = read_file_prefix(&abs, XMP_SCAN_PREFIX_BYTES).await;
                    if bytes.is_empty() {
                        return;
                    }
                    let mut sub = extract_xmp_subtype(&bytes);

                    // Re-extract dimensions for legacy rows missing width/height.
                    let (mut eff_w, mut eff_h) = (ph_w, ph_h);
                    if eff_w <= 0 || eff_h <= 0 {
                        let fname = abs
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_string();
                        let (rw, rh, _, _, _, _) =
                            extract_media_metadata_from_bytes_async(bytes.clone(), fname).await;
                        if rw > 0 && rh > 0 {
                            eff_w = rw;
                            eff_h = rh;
                            let _ =
                                sqlx::query("UPDATE photos SET width = ?, height = ? WHERE id = ?")
                                    .bind(eff_w)
                                    .bind(eff_h)
                                    .bind(&pid)
                                    .execute(&pool)
                                    .await;
                        }
                    }

                    apply_aspect_subtype_fallback(&mut sub, eff_w, eff_h);

                    if let Some(ref subtype) = sub.photo_subtype {
                        let res = sqlx::query(
                            "UPDATE photos \
                     SET photo_subtype = ?, burst_id = COALESCE(burst_id, ?) \
                     WHERE id = ? AND photo_subtype IS NULL",
                        )
                        .bind(subtype)
                        .bind(&sub.burst_id)
                        .bind(&pid)
                        .execute(&pool)
                        .await;
                        if let Ok(r) = res {
                            if r.rows_affected() > 0 {
                                updated.fetch_add(1, Ordering::Relaxed);
                                tracing::debug!(
                                    photo_id = %pid,
                                    subtype = %subtype,
                                    "[subtype-backfill] tagged photo"
                                );
                            }
                        }
                    }
                })
                .await;
            }
        })
        .buffer_unordered(4)
        .for_each(|_| async {})
        .await;

    let n = updated.load(Ordering::Relaxed);
    if n > 0 {
        tracing::info!(updated = n, "[subtype-backfill] complete");
    }
    n
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
        tracing::warn!("[xmp] Motion video too small: {} bytes", video_bytes.len());
        None
    }
}

// ── XMP helpers ─────────────────────────────────────────────────────────────

/// Extract an integer attribute value from XMP text.
/// Looks for patterns like `AttrName="12345"` or `AttrName='12345'`.
fn extract_xmp_int_attr(text: &str, attr_name: &str) -> Option<u64> {
    // Try pattern: AttrName="value"
    let pattern = format!("{attr_name}=\"");
    if let Some(pos) = text.find(&pattern) {
        let start = pos + pattern.len();
        let rest = &text[start..];
        if let Some(end) = rest.find('"') {
            return rest[..end].trim().parse().ok();
        }
    }
    // Try pattern: AttrName='value'
    let pattern = format!("{attr_name}='");
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
    let pattern = format!("{attr_name}=\"");
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
    let pattern = format!("{attr_name}='");
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

#[cfg(test)]
mod xmp_tests {
    //! Unit tests for the XMP subtype extractor.  These deliberately exercise
    //! the brittleness fixed under todo P0-6: single-quote attributes,
    //! varying namespace prefix casing, and `MotionPhoto`/`MicroVideo`
    //! values that are not the literal string `"1"`.
    //!
    //! Each test injects a JPEG byte stream containing an APP1/XMP packet
    //! and asserts the extractor produces the expected `SubtypeInfo`.

    use super::{apply_aspect_subtype_fallback, extract_xmp_subtype, SubtypeInfo};

    fn jpeg_with_xmp(xmp: &str) -> Vec<u8> {
        // SOI (FFD8) + APP1 with XMP packet + EOI (FFD9).
        let xmp_header = b"http://ns.adobe.com/xap/1.0/\x00";
        let mut payload = Vec::with_capacity(xmp_header.len() + xmp.len());
        payload.extend_from_slice(xmp_header);
        payload.extend_from_slice(xmp.as_bytes());
        let seg_len = (payload.len() + 2) as u16;
        let mut out = Vec::with_capacity(8 + payload.len());
        out.extend_from_slice(&[0xFF, 0xD8]); // SOI
        out.extend_from_slice(&[0xFF, 0xE1]); // APP1
        out.extend_from_slice(&seg_len.to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&[0xFF, 0xD9]); // EOI
        out
    }

    #[test]
    fn motion_photo_double_quote_value_1() {
        let xmp = r#"<x:xmpmeta xmlns:x='adobe:ns:meta/'>
            <rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'>
            <rdf:Description GCamera:MotionPhoto="1" GCamera:MotionVideoOffset="123" /></rdf:RDF></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("motion"));
        assert_eq!(info.motion_video_offset, Some(123));
    }

    #[test]
    fn motion_photo_single_quote_value_1() {
        // Same content, single-quote attribute style — must still match.
        let xmp = r#"<x:xmpmeta xmlns:x='adobe:ns:meta/'>
            <rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'>
            <rdf:Description GCamera:MotionPhoto='1' GCamera:MotionVideoOffset='456' /></rdf:RDF></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("motion"));
        assert_eq!(info.motion_video_offset, Some(456));
    }

    #[test]
    fn motion_photo_lowercase_namespace_prefix() {
        // Some Pixel firmware emits the lower-case prefix `gcamera:`.
        let xmp = r#"<x:xmpmeta><rdf:Description gcamera:MotionPhoto="1" /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("motion"));
    }

    #[test]
    fn motion_photo_zero_value_is_not_motion() {
        // Per XMP spec, `MotionPhoto="0"` means *not* a motion photo.
        let xmp = r#"<x:xmpmeta><rdf:Description GCamera:MotionPhoto="0" /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype, None);
        assert_eq!(info.motion_video_offset, None);
    }

    #[test]
    fn motion_photo_false_value_is_not_motion() {
        let xmp = r#"<x:xmpmeta><rdf:Description GCamera:MotionPhoto="false" /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype, None);
    }

    #[test]
    fn microvideo_old_schema() {
        // Camera:MicroVideo (older Pixel cameras).
        let xmp = r#"<x:xmpmeta><rdf:Description Camera:MicroVideo="1" Camera:MicroVideoOffset="789" /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("motion"));
        assert_eq!(info.motion_video_offset, Some(789));
    }

    #[test]
    fn panorama_equirectangular_single_quotes() {
        let xmp =
            r#"<x:xmpmeta><rdf:Description GPano:ProjectionType='equirectangular' /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("equirectangular"));
    }

    #[test]
    fn panorama_cylindrical_lowercase_prefix() {
        let xmp =
            r#"<x:xmpmeta><rdf:Description gpano:ProjectionType="cylindrical" /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("panorama"));
    }

    #[test]
    fn burst_id_single_quotes() {
        let xmp = r#"<x:xmpmeta><rdf:Description GCamera:BurstID='abc-123' /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("burst"));
        assert_eq!(info.burst_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn hdr_gainmap_detected() {
        let xmp = r#"<x:xmpmeta><rdf:Description hdrgm:Version="1.0" /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("hdr"));
    }

    #[test]
    fn burst_with_hdr_keeps_burst_id_and_burst_subtype() {
        // Pixel Ultra HDR burst frame: both markers present.  The frame must
        // stack with its burst group — burst wins over the hdr badge, and
        // burst_id must never be dropped.
        let xmp = r#"<x:xmpmeta><rdf:Description GCamera:BurstID="b-42" hdrgm:Version="1.0" /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("burst"));
        assert_eq!(info.burst_id.as_deref(), Some("b-42"));
    }

    #[test]
    fn motion_with_burst_id_keeps_both() {
        let xmp = r#"<x:xmpmeta><rdf:Description GCamera:MotionPhoto="1" GCamera:MotionVideoOffset="99" GCamera:BurstID="b-7" /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("motion"));
        assert_eq!(info.burst_id.as_deref(), Some("b-7"));
        assert_eq!(info.motion_video_offset, Some(99));
    }

    #[test]
    fn fisheye_projection_is_panorama() {
        let xmp = r#"<x:xmpmeta><rdf:Description GPano:ProjectionType="fisheye" /></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("panorama"));
    }

    #[test]
    fn motion_container_directory_schema_offset() {
        // Modern Pixel schema: no *Offset attribute; the video Item:Length
        // (plus Item:Padding) gives the trailer offset from EOF.
        let xmp = r#"<x:xmpmeta xmlns:Container="http://ns.google.com/photos/1.0/container/"
            xmlns:Item="http://ns.google.com/photos/1.0/container/item/">
            <rdf:Description GCamera:MotionPhoto="1" GCamera:MotionPhotoVersion="1">
            <Container:Directory><rdf:Seq>
            <rdf:li rdf:parseType="Resource"><Container:Item Item:Mime="image/jpeg" Item:Semantic="Primary" Item:Length="0" Item:Padding="0"/></rdf:li>
            <rdf:li rdf:parseType="Resource"><Container:Item Item:Mime="video/mp4" Item:Semantic="MotionPhoto" Item:Length="4061821" Item:Padding="0"/></rdf:li>
            </rdf:Seq></Container:Directory></rdf:Description></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.photo_subtype.as_deref(), Some("motion"));
        assert_eq!(info.motion_video_offset, Some(4061821));
    }

    #[test]
    fn motion_container_schema_with_padding() {
        let xmp = r#"<x:xmpmeta><rdf:Description GCamera:MotionPhoto="1">
            <Container:Item Item:Mime="image/jpeg" Item:Length="0"/>
            <Container:Item Item:Mime="video/mp4" Item:Length="1000" Item:Padding="8"/>
            </rdf:Description></x:xmpmeta>"#;
        let info = extract_xmp_subtype(&jpeg_with_xmp(xmp));
        assert_eq!(info.motion_video_offset, Some(1008));
    }

    #[test]
    fn no_xmp_no_subtype() {
        // Plain JPEG with no XMP packet.
        let bytes = vec![0xFF, 0xD8, 0xFF, 0xD9];
        let info = extract_xmp_subtype(&bytes);
        assert_eq!(info, SubtypeInfo::default());
    }

    #[test]
    fn aspect_fallback_equirectangular_2to1() {
        // True 360° photo sphere — 2:1 ratio AND ≥ 4000 px wide.
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 5760, 2880);
        assert_eq!(info.photo_subtype.as_deref(), Some("equirectangular"));
    }

    #[test]
    fn aspect_fallback_2to1_below_4k_is_panorama_not_equirect() {
        // 2:1 but only 3000 px wide — likely a regular wide stitch, not a
        // 360° sphere.  Treated as panorama.
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 3000, 1500);
        assert_eq!(info.photo_subtype.as_deref(), Some("panorama"));
    }

    #[test]
    fn aspect_fallback_wide_panorama() {
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 7000, 1000);
        assert_eq!(info.photo_subtype.as_deref(), Some("panorama"));
    }

    #[test]
    fn aspect_fallback_small_long_edge_is_skipped() {
        // Long edge < 2048 — not enough resolution to be a real panorama.
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 1200, 600);
        assert_eq!(info.photo_subtype, None);
    }

    #[test]
    fn aspect_fallback_skips_when_xmp_already_set() {
        let mut info = SubtypeInfo {
            photo_subtype: Some("motion".to_string()),
            ..SubtypeInfo::default()
        };
        apply_aspect_subtype_fallback(&mut info, 7000, 1000);
        assert_eq!(info.photo_subtype.as_deref(), Some("motion"));
    }

    #[test]
    fn aspect_fallback_skips_too_small() {
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 800, 200);
        assert_eq!(info.photo_subtype, None);
    }

    #[test]
    fn aspect_fallback_skips_normal_photo() {
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 4000, 3000);
        assert_eq!(info.photo_subtype, None);
    }

    #[test]
    fn aspect_fallback_handles_zero_dims() {
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 0, 0);
        assert_eq!(info.photo_subtype, None);
    }

    // ── New regression coverage for the lowered aspect threshold ────────

    #[test]
    fn aspect_fallback_18to1_is_normal_photo() {
        // 1.8:1 is a common cinematic / ultra-wide landscape ratio.  The
        // old fallback over-matched these as panoramas; the tightened
        // threshold (>= 2.0) correctly leaves them untagged.
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 5760, 3200); // 1.8:1
        assert_eq!(info.photo_subtype, None);
    }

    #[test]
    fn aspect_fallback_real_phone_panorama() {
        // Real stitched phone panoramas are typically ≥ 3:1.
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 9000, 2000); // 4.5:1
        assert_eq!(info.photo_subtype.as_deref(), Some("panorama"));
    }

    #[test]
    fn aspect_fallback_just_below_threshold_is_normal() {
        // 1.77 (≈ 16:9) is still a normal photo — guards against false
        // positives on widescreen captures and screenshots.
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 1920, 1080);
        assert_eq!(info.photo_subtype, None);
    }

    #[test]
    fn aspect_fallback_vertical_panorama() {
        // Samsung "vertical pano" — h/w ≥ 2.5.
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 1080, 4000); // h/w ≈ 3.7
        assert_eq!(info.photo_subtype.as_deref(), Some("panorama"));
    }

    #[test]
    fn aspect_fallback_portrait_video_is_not_panorama() {
        // 9:16 portrait video (h/w ≈ 1.78) must NOT be tagged.
        let mut info = SubtypeInfo::default();
        apply_aspect_subtype_fallback(&mut info, 2160, 3840);
        assert_eq!(info.photo_subtype, None);
    }
}
