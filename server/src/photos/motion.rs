//! Motion-photo (live photo) video trailer extraction and storage.
//!
//! One shared implementation for the three import paths (upload, scan
//! registration, scan retro-detection) that previously each carried their
//! own copy of this logic — and all three wrote the blob to an ad-hoc
//! `blobs/{id}.mp4` path that the serving code never looked at.
//!
//! Blobs are now written through [`crate::blobs::storage::write_blob`] so
//! the recorded `storage_path` is the single source of truth;
//! `serve_motion_video` reads that column instead of re-deriving a path.

use std::path::Path;

use sqlx::SqlitePool;
use uuid::Uuid;

/// Samsung SEF trailer tag: the embedded MP4 starts immediately after this
/// marker inside the trailer block appended past the JPEG EOI.
const SAMSUNG_MOTION_MARKER: &[u8] = b"MotionPhoto_Data";

/// Locate a Samsung motion-photo trailer when XMP carries no offset.
/// Returns the offset of the MP4 from end-of-file, like the XMP attributes.
pub(crate) fn find_samsung_motion_offset(data: &[u8]) -> Option<u64> {
    // The SEF block sits near EOF; bound the scan to the last 16 MB so a
    // pathological file can't make this quadratic-feeling.
    let start = data.len().saturating_sub(16 * 1024 * 1024);
    let hay = &data[start..];
    let pos = hay
        .windows(SAMSUNG_MOTION_MARKER.len())
        .rposition(|w| w == SAMSUNG_MOTION_MARKER)?;
    let mp4_start = start + pos + SAMSUNG_MOTION_MARKER.len();
    if mp4_start >= data.len() {
        return None;
    }
    Some((data.len() - mp4_start) as u64)
}

/// Extract the embedded MP4 from a motion photo's bytes and store it as a
/// `motion_video` blob linked to `photo_id`.
///
/// `offset_hint` is the XMP-declared trailer offset when known; when absent
/// (e.g. Samsung files whose XMP lacks `MicroVideoOffset`) the Samsung SEF
/// marker is searched as a fallback.
///
/// Returns the new blob id on success.  Every failure path logs — a motion
/// photo that silently loses its video is exactly the class of bug this
/// module exists to prevent.
pub(crate) async fn extract_and_store_motion_video(
    pool: &SqlitePool,
    storage_root: &Path,
    user_id: &str,
    photo_id: &str,
    file_bytes: &[u8],
    offset_hint: Option<u64>,
) -> Option<String> {
    let offset = match offset_hint.or_else(|| find_samsung_motion_offset(file_bytes)) {
        Some(o) => o,
        None => {
            tracing::warn!(
                photo_id = %photo_id,
                "Motion photo has no XMP offset and no Samsung trailer marker — video not extracted"
            );
            return None;
        }
    };

    let video_bytes = match super::metadata::extract_motion_video(file_bytes, offset) {
        Some(v) => v,
        None => {
            tracing::warn!(
                photo_id = %photo_id,
                offset,
                "Motion video extraction failed (offset out of range or trailer too small)"
            );
            return None;
        }
    };

    let blob_id = Uuid::new_v4().to_string();
    let rel_path =
        match crate::blobs::storage::write_blob(storage_root, user_id, &blob_id, &video_bytes)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(photo_id = %photo_id, error = %e, "Failed to write motion video blob");
                return None;
            }
        };

    let blob_size = video_bytes.len() as i64;
    let now = chrono::Utc::now().to_rfc3339();

    let insert = sqlx::query(
        "INSERT INTO blobs (id, user_id, blob_type, size_bytes, upload_time, storage_path) \
         VALUES (?, ?, 'motion_video', ?, ?, ?)",
    )
    .bind(&blob_id)
    .bind(user_id)
    .bind(blob_size)
    .bind(&now)
    .bind(&rel_path)
    .execute(pool)
    .await;

    if let Err(e) = insert {
        tracing::warn!(photo_id = %photo_id, error = %e, "Failed to register motion video blob row");
        let _ = crate::blobs::storage::delete_blob(storage_root, &rel_path).await;
        return None;
    }

    if let Err(e) = sqlx::query("UPDATE photos SET motion_video_blob_id = ? WHERE id = ?")
        .bind(&blob_id)
        .bind(photo_id)
        .execute(pool)
        .await
    {
        tracing::warn!(photo_id = %photo_id, error = %e, "Failed to link motion video blob to photo");
        return None;
    }

    tracing::info!(
        photo_id = %photo_id,
        blob_id = %blob_id,
        size = blob_size,
        "Extracted and stored motion video blob"
    );
    Some(blob_id)
}

#[cfg(test)]
mod tests {
    use super::find_samsung_motion_offset;

    #[test]
    fn samsung_marker_offset() {
        let mut data = vec![0u8; 100];
        data.extend_from_slice(b"MotionPhoto_Data");
        data.extend_from_slice(&[0, 0, 0, 24]);
        data.extend_from_slice(b"ftypmp42");
        // MP4 is the last 12 bytes
        assert_eq!(find_samsung_motion_offset(&data), Some(12));
    }

    #[test]
    fn no_marker_returns_none() {
        let data = vec![0u8; 256];
        assert_eq!(find_samsung_motion_offset(&data), None);
    }

    #[test]
    fn marker_at_eof_returns_none() {
        let mut data = vec![0u8; 32];
        data.extend_from_slice(b"MotionPhoto_Data");
        assert_eq!(find_samsung_motion_offset(&data), None);
    }
}
