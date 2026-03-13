//! Google Photos Takeout import and metadata management.
//!
//! Sub-modules:
//! - [`handlers`]      — Import metadata from JSON sidecars, upload sidecar files.
//! - [`takeout`]       — Scan and import from Google Photos Takeout directory structure.
//! - [`google_photos`] — Google Photos JSON sidecar parser.
//! - [`models`]        — Request/response DTOs for import endpoints.

pub mod google_photos;
pub mod handlers;
pub mod models;
pub mod takeout;
