//! Photo management — the core of Simple Photos.
//!
//! All media is encrypted — files are stored as opaque blobs (see [`crate::blobs`]);
//! the server never sees cleartext media.  The photos table and on-disk files
//! are used only by the autoscan/conversion pipeline.
//!
//! Key sub-modules:
//! - [`handlers`]        — List, serve, favorite, and crop endpoints for photos.
//! - [`upload`]          — Mobile client upload with content-hash deduplication.
//! - [`scan`]            — Filesystem scan, thumbnail & web-preview generation.
//! - [`convert`]         — Background media conversion task (MKV→MP4, HEIC→JPEG, etc.).
//! - [`encryption`]      — Encryption toggle, migration progress, and mark-encrypted.
//! - [`server_migrate`]  — Server-side parallel encryption migration pipeline.
//! - [`sync`]            — Encrypted-mode metadata sync for mobile gallery population.
//! - [`copies`]          — Photo duplication and edit-copy management.
//! - [`galleries`]       — Secure (password-protected) gallery CRUD.
//! - [`cleanup`]         — Remove plain originals after successful encryption.
//! - [`storage_stats`]   — Per-user and filesystem storage usage stats.
//! - [`metadata`]        — EXIF extraction (dimensions, GPS, camera model, date).
//! - [`utils`]           — Timestamp normalization and content hashing.

pub mod cleanup;
pub mod convert;
pub mod copies;
pub mod encryption;
pub mod galleries;
pub mod handlers;
pub mod metadata;
pub mod models;
pub mod scan;
pub mod server_migrate;
pub mod storage_stats;
pub mod sync;
pub mod upload;
pub mod utils;
