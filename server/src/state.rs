//! Shared application state, injected into every Axum handler via `State<AppState>`.
//!
//! All fields use `Arc` (or are internally `Arc`-wrapped) so cloning the struct
//! is cheap — Axum clones state into each handler invocation.

use arc_swap::ArcSwap;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use crate::config::AppConfig;
use crate::ratelimit::RateLimiters;

/// Shared state for all request handlers and background tasks.
#[derive(Clone)]
pub struct AppState {
    /// SQLite **write** connection pool — used for INSERT/UPDATE/DELETE
    /// operations and transactions.  Limited connections because SQLite
    /// allows only one concurrent writer.
    pub pool: SqlitePool,
    /// SQLite **read-only** connection pool — used for SELECT queries in
    /// request handlers.  Has many more connections than the write pool and
    /// uses `PRAGMA query_only = 1` to guarantee no accidental writes.
    /// This separation ensures gallery reads are never starved by concurrent
    /// uploads/backups writing to the database.
    pub read_pool: SqlitePool,
    /// Immutable server configuration loaded at startup.
    pub config: Arc<AppConfig>,
    /// In-memory per-IP rate limiters for auth endpoints (login, register, TOTP).
    pub rate_limiters: RateLimiters,
    /// Mutable storage root — can be changed at runtime via admin API.
    /// Uses ArcSwap for lock-free reads (only written by admin storage
    /// update, which is extremely rare). Every handler reads this on
    /// every request, so avoiding the async RwLock overhead matters.
    pub storage_root: Arc<ArcSwap<PathBuf>>,
    /// Mutex to serialize scan operations (manual scan, auto-scan, background
    /// autoscan).  Prevents concurrent scans from racing and creating
    /// duplicate photo entries even when the DB UNIQUE constraint exists.
    pub scan_lock: Arc<tokio::sync::Mutex<()>>,
}
