//! EXIF and media metadata extraction.
//!
//! Provides two entry points:
//! - [`extract_media_metadata`] — reads from a file path (used during scan).
//! - [`extract_media_metadata_from_bytes`] — reads from in-memory bytes
//!   (used during upload).
//!
//! Both extract: image dimensions (via `imagesize`), camera make/model, GPS
//! coordinates, and `DateTimeOriginal` (via the `exif` crate).
//!
//! Implementation is split across focused submodules:
//! - [`media`]   — EXIF/dimension extraction from files and byte buffers.
//! - [`subtype`] — XMP subtype detection, motion-video extraction, backfill.

mod media;
mod subtype;

pub(crate) use media::{
    extract_media_metadata_async, extract_media_metadata_from_bytes_async,
    repair_orientation_dimensions,
};
pub(crate) use subtype::{
    apply_aspect_subtype_fallback, backfill_photo_subtypes_all_users, extract_motion_video,
    extract_xmp_subtype, extract_xmp_subtype_async, read_file_prefix, XMP_SCAN_PREFIX_BYTES,
};

// Crate-wide API kept for completeness; currently only referenced from within
// this module's submodules, so the re-exports themselves are otherwise unused.
#[allow(unused_imports)]
pub(crate) use media::{extract_media_metadata, extract_media_metadata_from_bytes};
#[allow(unused_imports)]
pub(crate) use subtype::extract_xmp_subtype_from_file;

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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SubtypeInfo {
    /// Photo subtype: "motion", "panorama", "equirectangular", "hdr", "burst"
    pub photo_subtype: Option<String>,
    /// Burst group identifier (shared across shots in a burst)
    pub burst_id: Option<String>,
    /// Byte offset from end-of-file to the start of the embedded MP4
    /// (motion photos only — used to extract the video trailer)
    pub motion_video_offset: Option<u64>,
}
