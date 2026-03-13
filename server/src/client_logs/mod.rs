//! Client diagnostic log collection.
//!
//! Mobile clients submit debug logs (backup progress, errors, sync status)
//! that admins can review via the web UI. Logs are retained for 14 days.

pub mod handlers;
pub mod models;
