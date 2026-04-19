//! Photo management — the core of Simple Photos.
//!
//! All media is always encrypted — files are stored as opaque AES-256-GCM
//! blobs (see [`crate::blobs`]); the server never sees cleartext media.
//! The photos table and on-disk files are used only by the autoscan pipeline.
//!
//! Browser-native formats are supported directly.  Non-native formats
//! (HEIC, TIFF, MKV, WMA, etc.) are converted to browser-native equivalents
//! via FFmpeg during import — see [`crate::conversion`].
//!
//! Key sub-modules:
//! - [`handlers`]        — List, serve, favorite endpoints for photos.
//! - [`upload`]          — Mobile client upload with content-hash deduplication.
//! - [`scan`]            — Filesystem scan and thumbnail generation (pure Rust).
//! - [`encryption`]      — Encryption key storage endpoint.
//! - [`storage_stats`]   — Per-user and filesystem storage usage stats.
//! - [`metadata`]        — EXIF extraction (dimensions, GPS, camera model, date).
//! - [`utils`]           — Timestamp normalization and content hashing.

pub mod burst;
pub mod encryption;
pub mod handlers;
pub mod metadata;
pub mod metadata_edit;
pub mod models;
pub mod scan;
pub mod serve;
pub mod server_migrate;
pub mod server_migrate_encrypt;
pub mod storage_stats;
pub mod thumbnail;
pub mod upload;
pub mod utils;
pub mod web_preview;
