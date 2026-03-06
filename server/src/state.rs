use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{Notify, RwLock};

use crate::config::AppConfig;
use crate::ratelimit::RateLimiters;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<AppConfig>,
    pub rate_limiters: RateLimiters,
    /// Mutable storage root — can be changed at runtime via admin API.
    /// Initialised from config.storage.root on startup.
    pub storage_root: Arc<RwLock<PathBuf>>,
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
