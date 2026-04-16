//! Gallery engine — consolidates secure galleries, shared albums, and
//! encrypted sync into a single server-side module.
//!
//! ## Sub-modules
//!
//! - `secure`  — Secure (password-protected) gallery CRUD + item management
//! - `shared`  — Shared album CRUD, member/photo management
//! - `sync`    — Encrypted-sync endpoint for client→server photo metadata
//! - `models`  — Re-exports of model types used across gallery operations

pub mod secure;
pub mod shared;
pub mod sync;
pub mod models;
