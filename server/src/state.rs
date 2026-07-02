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
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Observable health of the background AI processor (item #16).
///
/// Lets operators and the client see that AI processing is alive and bounded
/// rather than silently wedged or crash-looping. All fields are updated by the
/// processor loop after every batch and surfaced via `GET /api/status/activity`.
#[derive(Debug, Default)]
pub struct AiHealth {
    /// Consecutive batch-level failures. Non-zero means the circuit breaker is
    /// backing the processor off; a persistently high value is an alert signal.
    pub consecutive_errors: AtomicU32,
    /// Unix seconds when the last batch completed (success or failure). `0`
    /// until the processor has run at least once.
    pub last_batch_unix: AtomicI64,
    /// Wall-clock duration of the last completed batch, in milliseconds.
    pub last_batch_ms: AtomicU64,
    /// Photos processed in the last batch.
    pub last_batch_photos: AtomicU64,
}

impl AiHealth {
    /// Record a successful batch: resets the error counter and stamps timing.
    pub fn record_success(&self, photos: usize, elapsed_ms: u64) {
        self.consecutive_errors.store(0, Ordering::Relaxed);
        self.last_batch_photos
            .store(photos as u64, Ordering::Relaxed);
        self.last_batch_ms.store(elapsed_ms, Ordering::Relaxed);
        self.stamp_now();
    }

    /// Record a failed batch: increments the error counter and stamps time.
    /// Returns the new consecutive-error count so the caller can size backoff.
    pub fn record_error(&self) -> u32 {
        let n = self.consecutive_errors.fetch_add(1, Ordering::Relaxed) + 1;
        self.stamp_now();
        n
    }

    fn stamp_now(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.last_batch_unix.store(now, Ordering::Relaxed);
    }
}

/// A real-time sync notification (item #11). Broadcast to a user's connected
/// clients so they promptly refetch changed albums/gallery data.
///
/// Intentionally tiny: it names *what* changed, not the change itself — clients
/// then pull the authoritative server state. **Conflict resolution is
/// timestamp-based / last-write-wins**: the pulled record's `updated_at` /
/// `taken_at` wins, so an out-of-order event never clobbers newer local edits.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncEvent {
    /// Owner the change applies to; SSE subscribers filter on this so a user
    /// only ever sees their own changes.
    pub user_id: String,
    /// Coarse entity kind: `"album"` | `"photo"` | `"trash"`.
    pub kind: String,
    /// Affected entity id (album id / photo id), or empty for a bulk change.
    pub entity_id: String,
    /// Server epoch-ms when the change happened.
    pub ts: i64,
}

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
    /// Broadcast channel for real-time album/gallery sync notifications
    /// ([`SyncEvent`]). The `/api/sync/events` SSE endpoint subscribes and
    /// forwards each user their own events so clients refetch within seconds
    /// (item #11). Capacity 256 — a lagging subscriber is told to full-resync.
    pub sync_tx: broadcast::Sender<SyncEvent>,
    /// Whether the storage backend (network drive, local disk) is currently
    /// reachable.  Set by the background storage health monitor which probes
    /// the storage root every 10 seconds.  Handlers check this before
    /// performing I/O to return 503 immediately rather than hanging on a
    /// stale mount.
    pub storage_available: Arc<AtomicBool>,
    /// Detected GPU hardware acceleration capability for video transcoding.
    /// Probed once at startup from FFmpeg and cached for the process lifetime.
    pub hw_accel: Arc<HwAccelCapability>,
    /// Set to `true` while the AI processor is actively running a batch
    /// (face detection, object detection, clustering, etc.).  Read by
    /// `GET /api/status/activity` so the web client can spin the profile
    /// avatar while server work is in progress.
    pub ai_active: Arc<AtomicBool>,
    /// Liveness / circuit-breaker telemetry for the AI processor (item #16).
    /// Surfaced by `GET /api/status/activity` so a wedged or crash-looping
    /// processor is observable instead of silently starving the queue.
    pub ai_health: Arc<AiHealth>,
    /// Set to `true` while the geo processor is actively backfilling
    /// reverse-geocoded city/state/country or year/month data.  Read by
    /// `GET /api/status/activity`.
    pub geo_active: Arc<AtomicBool>,
    /// Whether the offline GeoNames reverse-geocoding dataset is loadable.
    /// Starts `true` (optimistic — don't alarm before the first poll) and is
    /// set by the geo processor: `false` when geocoding is wanted but the
    /// dataset file is missing / unparseable, `true` once it loads.  Read by
    /// `GET /api/status/activity` so the client can show "location data
    /// unavailable" instead of a spinner that never resolves when the
    /// dataset isn't installed.
    pub geo_dataset_available: Arc<AtomicBool>,
    /// Set to `true` while the geo processor is actively downloading the
    /// GeoNames dataset at runtime (self-healing a failed install). Read by
    /// `GET /api/status/activity` so the client shows "downloading location
    /// data…" instead of the static "unavailable" notice.
    pub geo_dataset_downloading: Arc<AtomicBool>,
    /// Wakes the background geo processor *immediately* instead of waiting for
    /// its next poll tick (up to `geo.poll_interval_secs`, 5 min by default).
    ///
    /// Fired whenever new work appears that the user expects to see resolve
    /// promptly: enabling geolocation, uploading a GPS photo, or an auto-scan
    /// registering new files. Without this, flipping the toggle (or importing)
    /// sat idle for minutes with a frozen "0/N" banner that looked like a hang.
    pub geo_trigger: Arc<tokio::sync::Notify>,
}

impl AppState {
    /// Returns `true` if the storage backend is currently reachable.
    pub fn is_storage_available(&self) -> bool {
        self.storage_available.load(Ordering::Relaxed)
    }

    /// Broadcast a real-time sync notification for `user_id` (item #11).
    /// Best-effort: a send error just means no clients are subscribed right now,
    /// and the periodic background sync remains the fallback.
    pub fn emit_sync(&self, user_id: &str, kind: &str, entity_id: &str) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let _ = self.sync_tx.send(SyncEvent {
            user_id: user_id.to_string(),
            kind: kind.to_string(),
            entity_id: entity_id.to_string(),
            ts,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_health_error_counter_increments_and_resets() {
        let h = AiHealth::default();
        assert_eq!(h.consecutive_errors.load(Ordering::Relaxed), 0);

        // Failures accumulate and the returned count sizes the caller's backoff.
        assert_eq!(h.record_error(), 1);
        assert_eq!(h.record_error(), 2);
        assert_eq!(h.consecutive_errors.load(Ordering::Relaxed), 2);
        assert!(h.last_batch_unix.load(Ordering::Relaxed) > 0);

        // A successful batch trips the breaker back closed and records timing.
        h.record_success(8, 1234);
        assert_eq!(h.consecutive_errors.load(Ordering::Relaxed), 0);
        assert_eq!(h.last_batch_photos.load(Ordering::Relaxed), 8);
        assert_eq!(h.last_batch_ms.load(Ordering::Relaxed), 1234);
    }
}
