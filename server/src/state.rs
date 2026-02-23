use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

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
}
