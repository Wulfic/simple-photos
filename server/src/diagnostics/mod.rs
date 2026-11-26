//! Server diagnostics, audit log viewer, and external monitoring endpoints.
//!
//! Sub-modules:
//! - [`collect`]  — Shared metric collectors used by both admin and external endpoints.
//! - [`handlers`] — Admin-only diagnostics dashboard (storage, performance, logs).
//! - [`external`] — HTTP Basic Auth endpoints for server-to-server health checks.
//! - [`models`]   — Response DTOs for all diagnostics endpoints.
//! - [`inflight`] — Open-request registry + decrypt-stream gauge (N2).
//! - [`watchdog`] — OS-thread liveness watchdog that logs why HTTP died (N2).

pub mod collect;
pub mod external;
pub mod handlers;
pub mod inflight;
pub mod models;
pub mod watchdog;
