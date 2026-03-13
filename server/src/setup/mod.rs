//! Server setup and administration.
//!
//! Sub-modules:
//! - [`handlers`] — First-run setup wizard (status, init, pair, discover).
//! - [`admin`]    — User management CRUD (create, list, delete, role, password, 2FA).
//! - [`import`]   — Server-side directory scan for client-driven import.
//! - [`port`]     — Server port configuration and restart.
//! - [`ssl`]      — TLS certificate configuration.
//! - [`storage`]  — Storage root and directory browsing.

pub mod admin;
pub mod handlers;
pub mod import;
pub mod port;
pub mod ssl;
pub mod storage;
