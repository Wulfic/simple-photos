//! Gallery engine — consolidates secure galleries, shared albums, and
//! encrypted sync into a single server-side module.
//!
//! ## Sub-modules
//!
//! - `secure`       — Secure (password-protected) gallery CRUD + item management
//! - `secure_token` — Generation/verification of secure-gallery unlock tokens
//! - `access`       — Serve-path gate for secure items (token extractor + check)
//! - `shared`       — Shared album CRUD, member/photo management
//! - `eligibility`  — The ONE definition of "is this photo in the gallery feed?"
//! - `sync`         — Encrypted-sync endpoint for client→server photo metadata
//! - `summary`      — Cheap precomputed gallery count summary (smart-album badges)
//! - `models`       — Re-exports of model types used across gallery operations

pub mod access;
pub mod eligibility;
pub mod models;
pub mod secure;
pub mod secure_token;
pub mod shared;
pub mod summary;
pub mod sync;
