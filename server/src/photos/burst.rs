//! Burst photo detection by timestamp proximity.
//!
//! Groups photos taken within a short time window (default 2 seconds)
//! from the same camera into burst sequences. Assigns a shared `burst_id`
//! to all photos in a burst group.
//!
//! Complements the XMP BurstID detection in `metadata.rs` — this module
//! handles cameras that don't write BurstID to XMP metadata.
//!
//! ## Guard rails against false positives
//!
//! `taken_at` falls back to the file's mtime when a photo carries no EXIF
//! `DateTimeOriginal` (see scan/upload registration).  Bulk-copied or
//! restored files share mtimes down to the second, so naive timestamp
//! grouping stacks completely unrelated screenshots, downloads — even
//! videos — into fake bursts (observed live: a "burst" of .mp4/.webm/.gif
//! files).  Therefore candidates must:
//!
//!   * be still photos (`media_type = 'photo'`), and
//!   * carry a real camera model — cameras that write Make/Model also
//!     write `DateTimeOriginal`, so this excludes mtime-derived rows.
//!
//! ## Precedence with XMP-based detection
//!
//! XMP-based burst grouping (set during upload/scan from
//! `GCamera:BurstID`) ALWAYS takes precedence over timestamp-based
//! grouping.  This is enforced in two places:
//!
//!   1. The candidate `SELECT` filters `burst_id IS NULL AND
//!      photo_subtype IS NULL` — photos already grouped by XMP, or
//!      already classified as motion/panorama/hdr/etc., are skipped.
//!   2. The `UPDATE` has `WHERE id = ?2 AND burst_id IS NULL` so a
//!      concurrent XMP write between the SELECT and UPDATE cannot be
//!      clobbered.

use sqlx::SqlitePool;
use tracing;

/// Maximum gap between consecutive photos to be considered a burst (seconds).
const BURST_GAP_SECS: f64 = 2.0;

/// Minimum number of photos to form a burst group.
const MIN_BURST_SIZE: usize = 2;

/// Maximum distance between consecutive frames of a burst.  Bursts are shot
/// from one spot; 1 km generously covers a fast vehicle between frames while
/// rejecting same-timestamp photos from different places (stuck clocks).
const MAX_BURST_SPREAD_KM: f64 = 1.0;

/// Detect and assign burst groups for a user based on timestamp proximity.
///
/// Only processes still photos that don't already have a `burst_id`, have a
/// valid `taken_at` timestamp, and have an EXIF camera model (see module
/// docs for why). Groups photos from the same camera model within
/// `BURST_GAP_SECS` of each other.
pub async fn detect_bursts_for_user(pool: &SqlitePool, user_id: &str) -> anyhow::Result<u64> {
    // Fetch photos eligible for timestamp-based burst grouping.
    // Exclude photos that already have a burst_id (already grouped) or a
    // photo_subtype (already categorised as motion/panorama/hdr/etc. —
    // overwriting their subtype would be incorrect).
    let photos: Vec<(String, String, Option<String>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT id, taken_at, camera_model, latitude, longitude FROM photos \
         WHERE user_id = ?1 AND burst_id IS NULL AND photo_subtype IS NULL \
         AND taken_at IS NOT NULL \
         AND media_type = 'photo' \
         AND camera_model IS NOT NULL AND camera_model != '' \
         AND id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
         ORDER BY camera_model, taken_at ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    if photos.is_empty() {
        return Ok(0);
    }

    let mut groups_created = 0u64;
    let mut current_group: Vec<String> = Vec::new();
    let mut prev_time: Option<f64> = None;
    let mut prev_camera: Option<String> = None;
    let mut prev_coords: Option<(f64, f64)> = None;

    for (photo_id, taken_at, camera_model, lat, lon) in &photos {
        let ts = match parse_timestamp(taken_at) {
            Some(t) => t,
            None => {
                flush_group(pool, user_id, &mut current_group, &mut groups_created).await?;
                prev_time = None;
                prev_camera = None;
                prev_coords = None;
                continue;
            }
        };

        let cam = camera_model.as_deref().unwrap_or("");
        let coords = match (lat, lon) {
            (Some(la), Some(lo)) => Some((*la, *lo)),
            _ => None,
        };

        let same_camera = match &prev_camera {
            Some(pc) => pc == cam,
            None => true,
        };

        let within_gap = match prev_time {
            Some(pt) => (ts - pt).abs() <= BURST_GAP_SECS,
            None => false,
        };

        // Spatial coherence: a burst is shot from one spot.  Two photos a
        // second apart but kilometres apart are clock artifacts (cameras
        // with a stuck/default date stamp every file with the same
        // timestamp).  Photos without GPS can't be disproven and pass.
        let spatially_coherent = match (prev_coords, coords) {
            (Some((pla, plo)), Some((la, lo))) => {
                crate::geo::geocoder::haversine_km(pla, plo, la, lo) <= MAX_BURST_SPREAD_KM
            }
            _ => true,
        };

        if same_camera && within_gap && spatially_coherent {
            // Extend current group
            current_group.push(photo_id.clone());
        } else {
            // Flush previous group and start new one
            flush_group(pool, user_id, &mut current_group, &mut groups_created).await?;
            current_group.push(photo_id.clone());
        }

        prev_time = Some(ts);
        prev_camera = Some(cam.to_string());
        prev_coords = coords;
    }

    // Flush final group
    flush_group(pool, user_id, &mut current_group, &mut groups_created).await?;

    if groups_created > 0 {
        tracing::info!(
            user_id = %user_id,
            burst_groups = groups_created,
            "Burst detection: grouped photos by timestamp"
        );
    }

    Ok(groups_created)
}

/// Flush a burst group: if it has enough photos, assign a shared burst_id.
///
/// All rows of a group are written in one transaction so a crash mid-group
/// can't leave a half-tagged burst (which would render as a stack missing
/// frames in the gallery).
async fn flush_group(
    pool: &SqlitePool,
    _user_id: &str,
    group: &mut Vec<String>,
    count: &mut u64,
) -> anyhow::Result<()> {
    if group.len() >= MIN_BURST_SIZE {
        let burst_id = format!("burst-{}", uuid::Uuid::new_v4());

        let mut tx = pool.begin().await?;
        for photo_id in group.iter() {
            sqlx::query(
                "UPDATE photos SET burst_id = ?1, photo_subtype = 'burst' \
                 WHERE id = ?2 AND burst_id IS NULL",
            )
            .bind(&burst_id)
            .bind(photo_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        tracing::debug!(
            burst_id = %burst_id,
            photos = group.len(),
            "Burst detection: created burst group"
        );

        *count += 1;
    }

    group.clear();
    Ok(())
}

/// Parse an ISO 8601 timestamp string to seconds since epoch.
fn parse_timestamp(ts: &str) -> Option<f64> {
    // Handle common formats: "2024-01-15T14:30:00Z", "2024-01-15 14:30:00", etc.
    let cleaned = ts
        .trim()
        .replace('T', " ")
        .trim_end_matches('Z')
        .to_string();

    // Try chrono parsing
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&cleaned, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(
            dt.and_utc().timestamp() as f64 + dt.and_utc().timestamp_subsec_nanos() as f64 / 1e9,
        );
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&cleaned, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp() as f64);
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&cleaned, "%Y:%m:%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp() as f64);
    }

    None
}
