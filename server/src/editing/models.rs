//! Shared editing data types — single source of truth for crop/edit metadata.
//!
//! Both the "Save" path (metadata-only, `PUT /crop`) and the "Save Copy" path
//! (rendered duplicate, `POST /duplicate`) use the same [`CropMeta`] struct to
//! represent edits.  This eliminates the duplicated definitions that previously
//! lived in `photos/copies.rs` and `photos/render.rs`.

use serde::Deserialize;

use crate::error::AppError;
use crate::sanitize;

/// Parsed edit parameters.  All fields are optional so partial metadata is
/// handled gracefully — a missing field means "use the default/neutral value".
///
/// The JSON keys match what the client editor produces:
/// ```json
/// {
///   "x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5,
///   "rotate": 90, "brightness": 20,
///   "trimStart": 1.5, "trimEnd": 8.0
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct CropMeta {
    /// Left edge of crop rect, 0–1 fraction of original width.
    pub x: Option<f64>,
    /// Top edge of crop rect, 0–1 fraction of original height.
    pub y: Option<f64>,
    /// Width of crop rect, 0–1 fraction of original width.  Default 1.0.
    pub width: Option<f64>,
    /// Height of crop rect, 0–1 fraction of original height.  Default 1.0.
    pub height: Option<f64>,
    /// Clockwise rotation in degrees.  Only 0 / 90 / 180 / 270 are supported.
    pub rotate: Option<f64>,
    /// Brightness adjustment, -100 (darkest) to +100 (brightest).  Default 0.
    pub brightness: Option<f64>,
    /// Trim start in seconds.  Omit or 0 = start of file.
    #[serde(rename = "trimStart")]
    pub trim_start: Option<f64>,
    /// Trim end in seconds.  Omit or 0 = end of file.
    #[serde(rename = "trimEnd")]
    pub trim_end: Option<f64>,
}

impl CropMeta {
    /// Parse a JSON string into a `CropMeta`, returning `None` for invalid JSON.
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }

    /// Whether a non-trivial crop region is specified (not full-frame).
    pub fn has_crop(&self) -> bool {
        self.width.unwrap_or(1.0) < 0.999
            || self.height.unwrap_or(1.0) < 0.999
            || self.x.unwrap_or(0.0) > 0.001
            || self.y.unwrap_or(0.0) > 0.001
    }

    /// Whether a non-zero rotation is specified.
    pub fn has_rotation(&self) -> bool {
        self.rotate.unwrap_or(0.0).abs() > 0.5
    }

    /// Whether a non-zero brightness adjustment is specified.
    pub fn has_brightness(&self) -> bool {
        self.brightness.unwrap_or(0.0).abs() > 0.5
    }

    /// Whether a non-trivial trim start is specified.
    pub fn has_trim_start(&self) -> bool {
        self.trim_start.unwrap_or(0.0) > 0.01
    }

    /// Whether a non-trivial trim end is specified (and end > start).
    pub fn has_trim_end(&self) -> bool {
        let ts = self.trim_start.unwrap_or(0.0);
        let te = self.trim_end.unwrap_or(0.0);
        te > 0.01 && te > ts + 0.01
    }

    /// Whether any trim (start or end) is specified.
    pub fn has_trim(&self) -> bool {
        self.has_trim_start() || self.has_trim_end()
    }

    /// Whether this rotation swaps width↔height (90° or 270°).
    pub fn rotation_swaps_dimensions(&self) -> bool {
        let rot = ((self.rotate.unwrap_or(0.0) as i32).rem_euclid(360)) as u32;
        rot == 90 || rot == 270
    }

    /// Rotation normalised to 0/90/180/270.
    pub fn rotation_degrees(&self) -> u32 {
        ((self.rotate.unwrap_or(0.0) as i32).rem_euclid(360)) as u32
    }

    /// Whether any video filter is needed (crop, rotate, or brightness).
    pub fn needs_video_filter(&self) -> bool {
        self.has_crop() || self.has_rotation() || self.has_brightness()
    }
}

/// Validate and sanitize a raw `crop_metadata` JSON string from the client.
///
/// Returns the sanitized JSON string on success.  Rejects strings that exceed
/// `max_len` or are not valid JSON objects.
pub fn validate_crop_json(raw: &str, max_len: usize) -> Result<String, AppError> {
    let sanitized = sanitize::sanitize_freeform(raw, max_len);
    if serde_json::from_str::<serde_json::Value>(&sanitized).is_err() {
        return Err(AppError::BadRequest(
            "crop_metadata must be valid JSON".into(),
        ));
    }
    Ok(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_frame_no_edits() {
        let m = CropMeta::from_json(r#"{"x":0,"y":0,"width":1,"height":1,"rotate":0,"brightness":0}"#).unwrap();
        assert!(!m.has_crop());
        assert!(!m.has_rotation());
        assert!(!m.has_brightness());
        assert!(!m.has_trim());
        assert!(!m.needs_video_filter());
        assert!(!m.rotation_swaps_dimensions());
    }

    #[test]
    fn crop_detected() {
        let m = CropMeta::from_json(r#"{"x":0.1,"y":0.2,"width":0.6,"height":0.5}"#).unwrap();
        assert!(m.has_crop());
        assert!(m.needs_video_filter());
    }

    #[test]
    fn rotation_90_swaps() {
        let m = CropMeta::from_json(r#"{"rotate":90}"#).unwrap();
        assert!(m.has_rotation());
        assert!(m.rotation_swaps_dimensions());
        assert_eq!(m.rotation_degrees(), 90);
    }

    #[test]
    fn rotation_180_no_swap() {
        let m = CropMeta::from_json(r#"{"rotate":180}"#).unwrap();
        assert!(m.has_rotation());
        assert!(!m.rotation_swaps_dimensions());
    }

    #[test]
    fn rotation_270_swaps() {
        let m = CropMeta::from_json(r#"{"rotate":270}"#).unwrap();
        assert!(m.rotation_swaps_dimensions());
        assert_eq!(m.rotation_degrees(), 270);
    }

    #[test]
    fn brightness_detected() {
        let m = CropMeta::from_json(r#"{"brightness":42}"#).unwrap();
        assert!(m.has_brightness());
        assert!(m.needs_video_filter());
    }

    #[test]
    fn trim_detected() {
        let m = CropMeta::from_json(r#"{"trimStart":1.5,"trimEnd":8.0}"#).unwrap();
        assert!(m.has_trim_start());
        assert!(m.has_trim_end());
        assert!(m.has_trim());
    }

    #[test]
    fn invalid_json_returns_none() {
        assert!(CropMeta::from_json("not json").is_none());
        assert!(CropMeta::from_json("").is_none());
    }

    #[test]
    fn partial_metadata() {
        let m = CropMeta::from_json(r#"{"brightness":10}"#).unwrap();
        assert!(m.x.is_none());
        assert!(!m.has_crop());
        assert!(!m.has_rotation());
        assert!(m.has_brightness());
    }
}
