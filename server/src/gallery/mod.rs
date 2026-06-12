//! Gallery engine — consolidates secure galleries, shared albums, and
//! encrypted sync into a single server-side module.
//!
//! ## Sub-modules
//!
//! - `secure`       — Secure (password-protected) gallery CRUD + item management
//! - `secure_token` — Generation/verification of secure-gallery unlock tokens
//! - `access`       — Serve-path gate for secure items (token extractor + check)
//! - `shared`       — Shared album CRUD, member/photo management
//! - `sync`         — Encrypted-sync endpoint for client→server photo metadata
//! - `models`       — Re-exports of model types used across gallery operations

pub mod access;
pub mod models;
pub mod secure;
pub mod secure_token;
pub mod shared;
pub mod sync;
