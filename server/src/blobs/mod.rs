//! Encrypted blob storage layer.
//!
//! Blobs are opaque, encrypted byte payloads stored on disk in a sharded
//! directory tree (`blobs/{user_prefix}/{user_id}/{blob_prefix}/{blob_id}.bin`).
//! The server never decrypts them — encryption/decryption happens client-side.
//!
//! Sub-modules:
//! - [`handlers`] — Upload, download (with HTTP Range support), list, delete.
//! - [`storage`]  — Filesystem read/write/delete helpers and path builders.
//! - [`models`]   — `BlobRecord` and API response types.

pub mod download;
pub mod handlers;
pub mod models;
pub mod storage;
