//! Photo management — the core of Simple Photos.
//!
//! All media is always encrypted — files are stored as opaque AES-256-GCM
//! blobs (see [`crate::blobs`]); the server never sees cleartext media.
//! The photos table and on-disk files are used only by the autoscan pipeline.
//!
//! Only browser-native formats are supported; no server-side conversion is
//! performed.
//!
//! Key sub-modules:
//! - [`handlers`]        — List, serve, favorite, and crop endpoints for photos.
//! - [`upload`]          — Mobile client upload with content-hash deduplication.
//! - [`scan`]            — Filesystem scan and thumbnail generation (pure Rust).
//! - [`encryption`]      — Encryption key storage endpoint.
//! - [`sync`]            — Photo metadata sync for mobile gallery population.
//! - [`copies`]          — Photo duplication and edit-copy management.
//! - [`galleries`]       — Secure (password-protected) gallery CRUD.
//! - [`storage_stats`]   — Per-user and filesystem storage usage stats.
//! - [`metadata`]        — EXIF extraction (dimensions, GPS, camera model, date).
//! - [`utils`]           — Timestamp normalization and content hashing.

pub mod copies;
pub mod encryption;
pub mod galleries;
pub mod handlers;
pub mod metadata;
pub mod models;
pub mod scan;
pub mod storage_stats;
pub mod sync;
pub mod upload;
pub mod utils;
