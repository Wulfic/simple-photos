//! Shared application state, injected into every Axum handler via `State<AppState>`.
//!
//! All fields use `Arc` (or are internally `Arc`-wrapped) so cloning the struct
//! is cheap — Axum clones state into each handler invocation.

use arc_swap::ArcSwap;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{Notify, RwLock};

use crate::config::AppConfig;
use crate::ratelimit::RateLimiters;

/// Shared state for all request handlers and background tasks.
#[derive(Clone)]
pub struct AppState {
    /// SQLite connection pool — shared across all handlers and background tasks.
    pub pool: SqlitePool,
    /// Immutable server configuration loaded at startup.
    pub config: Arc<AppConfig>,
    /// In-memory per-IP rate limiters for auth endpoints (login, register, TOTP).
    pub rate_limiters: RateLimiters,
    /// Mutable storage root — can be changed at runtime via admin API.
    /// Uses ArcSwap for lock-free reads (only written by admin storage
    /// update, which is extremely rare). Every handler reads this on
    /// every request, so avoiding the async RwLock overhead matters.
    pub storage_root: Arc<ArcSwap<PathBuf>>,
    /// Notify handle to wake the background conversion task immediately
    /// (e.g. after a scan or upload completes).
    pub convert_notify: Arc<Notify>,
    /// True while the background converter is actively processing files.
    /// Read by the conversion-status endpoint so the client banner stays
    /// visible even when the DB-driven pending count drops to 0 mid-pass
    /// (e.g. because encryption set encrypted_blob_id on those rows).
    pub conversion_active: Arc<AtomicBool>,
    /// Temporarily holds the AES-256 encryption key during and shortly after
    /// migration, so the background converter can decrypt encrypted blobs,
    /// convert media to web-compatible formats, and re-encrypt with the
    /// converted data. Cleared automatically after a grace period.
    pub encryption_key: Arc<RwLock<Option<[u8; 32]>>>,
}
