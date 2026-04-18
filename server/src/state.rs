//! Shared application state, injected into every Axum handler via `State<AppState>`.
//!
//! All fields use `Arc` (or are internally `Arc`-wrapped) so cloning the struct
//! is cheap — Axum clones state into each handler invocation.

use crate::config::AppConfig;
use crate::ratelimit::RateLimiters;
use crate::transcode::HwAccelCapability;
use arc_swap::ArcSwap;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// A serialised audit log entry broadcast to SSE subscribers and backup forwarders.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditBroadcast {
    pub id: String,
    pub event_type: String,
    pub user_id: Option<String>,
    pub ip_address: String,
    pub user_agent: String,
    pub details: String,
    pub created_at: String,
    pub source_server: Option<String>,
}

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
    /// Broadcast channel for real-time audit log events.
    /// SSE subscribers and backup log forwarders listen on this channel.
    /// Capacity of 256 — lagging receivers simply miss old entries (they
    /// can always fetch history via the REST endpoint).
    pub audit_tx: broadcast::Sender<AuditBroadcast>,
    /// Whether the storage backend (network drive, local disk) is currently
    /// reachable.  Set by the background storage health monitor which probes
    /// the storage root every 10 seconds.  Handlers check this before
    /// performing I/O to return 503 immediately rather than hanging on a
    /// stale mount.
    pub storage_available: Arc<AtomicBool>,
    /// Detected GPU hardware acceleration capability for video transcoding.
    /// Probed once at startup from FFmpeg and cached for the process lifetime.
    pub hw_accel: Arc<HwAccelCapability>,
}

impl AppState {
    /// Returns `true` if the storage backend is currently reachable.
    pub fn is_storage_available(&self) -> bool {
        self.storage_available.load(Ordering::Relaxed)
    }
}
