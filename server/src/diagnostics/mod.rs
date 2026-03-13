//! Server diagnostics, audit log viewer, and external monitoring endpoints.
//!
//! Sub-modules:
//! - [`handlers`] — Admin-only diagnostics dashboard (storage, performance, logs).
//! - [`external`] — HTTP Basic Auth endpoints for server-to-server health checks.
//! - [`models`]   — Response DTOs for all diagnostics endpoints.

pub mod external;
pub mod handlers;
pub mod models;
