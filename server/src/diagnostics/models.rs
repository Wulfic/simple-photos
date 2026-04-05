//! DTOs for server diagnostics, metrics, and audit-log responses.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Diagnostics configuration — controls whether expensive metrics collection runs.
#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticsConfig {
    /// Master toggle for server-side diagnostics collection (disk walks, table counts, etc.)
    pub diagnostics_enabled: bool,
    /// Whether web and mobile clients should collect and send diagnostic logs
    pub client_diagnostics_enabled: bool,
}

/// Top-level diagnostics response combining all server metrics.
#[derive(Debug, Serialize)]
pub struct DiagnosticsResponse {
    /// Whether diagnostics collection is enabled
    pub enabled: bool,
    pub server: ServerInfo,
    pub database: DatabaseStats,
    pub storage: StorageStats,
    pub users: UserStats,
    pub photos: PhotoStats,
    pub audit: AuditSummary,
    pub client_logs: ClientLogSummary,
    pub backup: BackupSummary,
    pub performance: PerformanceStats,
    /// How long each diagnostics section took to collect (ms)
    pub timings: CollectionTimings,
}

/// Lightweight response returned when diagnostics collection is disabled.
/// Still provides basic server identity info without expensive queries.
#[derive(Debug, Serialize)]
pub struct DisabledDiagnosticsResponse {
    pub enabled: bool,
    pub server: BasicServerInfo,
    pub message: String,
}

/// Minimal server info returned when diagnostics are disabled.
#[derive(Debug, Serialize)]
pub struct BasicServerInfo {
    pub version: String,
    pub uptime_seconds: u64,
    pub started_at: String,
}

#[derive(Debug, Serialize)]
pub struct ServerInfo {
    pub version: String,
    pub uptime_seconds: u64,
    pub rust_version: String,
    pub os: String,
    pub arch: String,
    /// Resident memory in bytes (Linux only, 0 elsewhere)
    pub memory_rss_bytes: u64,
    /// Process CPU time in seconds (user + system)
    pub cpu_seconds: f64,
    pub pid: u32,
    pub storage_root: String,
    pub db_path: String,
    pub tls_enabled: bool,
    pub max_blob_size_mb: u64,
    pub started_at: String,
    /// Number of OS threads in use by this process (Linux only, 0 elsewhere)
    pub thread_count: u64,
    /// Number of open file descriptors (Linux only, 0 elsewhere)
    pub open_fds: u64,
    /// System load averages [1min, 5min, 15min] (Linux/Unix only)
    pub load_average: [f64; 3],
}

#[derive(Debug, Serialize)]
pub struct DatabaseStats {
    pub size_bytes: u64,
    pub wal_size_bytes: u64,
    pub table_counts: HashMap<String, i64>,
    pub journal_mode: String,
    pub page_size: i64,
    pub page_count: i64,
    pub freelist_count: i64,
}

#[derive(Debug, Serialize)]
pub struct StorageStats {
    /// Total bytes consumed by photo/blob files
    pub total_bytes: u64,
    /// Total number of files on disk
    pub file_count: u64,
    /// Disk total/available (from statvfs on Linux)
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
    pub disk_used_percent: f64,
}

#[derive(Debug, Serialize)]
pub struct UserStats {
    pub total_users: i64,
    pub admin_count: i64,
    pub totp_enabled_count: i64,
}

#[derive(Debug, Serialize)]
pub struct PhotoStats {
    pub total_photos: i64,
    pub encrypted_count: i64,
    pub total_file_bytes: i64,
    pub total_thumb_bytes: i64,
    pub photos_with_thumbs: i64,
    pub photos_by_media_type: HashMap<String, i64>,
    pub oldest_photo: Option<String>,
    pub newest_photo: Option<String>,
    pub favorited_count: i64,
    pub tagged_count: i64,
}

#[derive(Debug, Serialize)]
pub struct AuditSummary {
    pub total_entries: i64,
    pub entries_last_24h: i64,
    pub entries_last_7d: i64,
    pub events_by_type: HashMap<String, i64>,
    pub recent_failures: Vec<AuditFailureEntry>,
}

#[derive(Debug, Serialize)]
pub struct AuditFailureEntry {
    pub event_type: String,
    pub ip_address: String,
    pub user_agent: String,
    pub created_at: String,
    pub details: String,
}

#[derive(Debug, Serialize)]
pub struct ClientLogSummary {
    pub total_entries: i64,
    pub entries_last_24h: i64,
    pub entries_last_7d: i64,
    pub by_level: HashMap<String, i64>,
    pub unique_sessions: i64,
}

#[derive(Debug, Serialize)]
pub struct BackupSummary {
    pub server_count: i64,
    pub total_sync_logs: i64,
    pub last_sync_at: Option<String>,
    /// Per-server details including their pushed diagnostics and recent sync logs
    pub servers: Vec<BackupServerDetail>,
}

/// Detailed info for a single backup server, including its latest diagnostics push.
#[derive(Debug, Serialize)]
pub struct BackupServerDetail {
    pub id: String,
    pub name: String,
    pub address: String,
    pub enabled: bool,
    pub sync_frequency_hours: i64,
    pub last_sync_at: Option<String>,
    pub last_sync_status: String,
    pub last_sync_error: Option<String>,
    /// The most recent diagnostics report pushed by this backup server
    pub last_diagnostics: Option<serde_json::Value>,
    pub last_diagnostics_at: Option<String>,
    /// Most recent sync log entries for this server
    pub recent_sync_logs: Vec<SyncLogBrief>,
}

/// Compact sync log entry for the diagnostics overview.
#[derive(Debug, Serialize)]
pub struct SyncLogBrief {
    pub id: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub photos_synced: i64,
    pub bytes_synced: i64,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PerformanceStats {
    /// Single-sample round-trip latency for `SELECT 1` (ms). Not averaged.
    pub db_ping_ms: f64,
    /// SQLite cache hit ratio (if available)
    pub cache_hit_ratio: Option<f64>,
    /// SQLite cache size in KiB (PRAGMA cache_size)
    pub cache_size_kib: i64,
    /// WAL checkpoint info — pages written vs checkpointed
    pub wal_checkpoint: Option<WalCheckpointInfo>,
    /// SQLite compile-time options (e.g. THREADSAFE, ENABLE_FTS5)
    pub compile_options: Vec<String>,
    /// Number of active read pool connections
    pub read_pool_size: u32,
    /// Number of active write pool connections
    pub write_pool_size: u32,
    /// Read pool idle connections
    pub read_pool_idle: u32,
    /// Write pool idle connections
    pub write_pool_idle: u32,
}

/// WAL checkpoint status from PRAGMA wal_checkpoint.
#[derive(Debug, Serialize)]
pub struct WalCheckpointInfo {
    /// Non-zero if a checkpoint was blocked by a concurrent reader/writer
    pub busy: i64,
    /// Size of the WAL in pages
    pub log_pages: i64,
    /// Number of pages successfully checkpointed
    pub checkpointed_pages: i64,
}

/// Timing data for each diagnostics collection phase.
#[derive(Debug, Serialize)]
pub struct CollectionTimings {
    /// Total wall-clock time to collect all diagnostics (ms)
    pub total_ms: f64,
    pub server_ms: f64,
    pub database_ms: f64,
    pub storage_ms: f64,
    pub users_ms: f64,
    pub photos_ms: f64,
    pub audit_ms: f64,
    pub client_logs_ms: f64,
    pub backup_ms: f64,
    pub performance_ms: f64,
}

// ── Audit log listing models ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AuditLogEntry {
    pub id: String,
    pub event_type: String,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub ip_address: String,
    pub user_agent: String,
    pub details: String,
    pub created_at: String,
    pub source_server: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditLogListResponse {
    pub logs: Vec<AuditLogEntry>,
    pub next_cursor: Option<String>,
    pub total: i64,
}

/// Query parameters for filtering and paginating the security audit log.
#[derive(Debug, serde::Deserialize)]
pub struct AuditLogParams {
    pub event_type: Option<String>,
    pub user_id: Option<String>,
    pub ip_address: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: Option<u32>,
    /// Filter by source server name.  `"local"` = only this server's events.
    pub source_server: Option<String>,
}

// ── Server log listing models ─────────────────────────────────────────────

/// Wrapper for the toggle request body
#[derive(Debug, Deserialize)]
pub struct UpdateDiagnosticsConfigRequest {
    pub diagnostics_enabled: Option<bool>,
    pub client_diagnostics_enabled: Option<bool>,
}

// ── External API response models ──────────────────────────────────────────

/// Lightweight health check for external monitoring systems.
#[derive(Debug, Serialize)]
pub struct ExternalHealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub started_at: String,
    pub memory_rss_bytes: u64,
    pub cpu_seconds: f64,
    pub db_ping_ms: f64,
    pub disk_used_percent: f64,
    pub total_photos: i64,
    pub total_users: i64,
}

/// Storage-focused response for capacity monitoring.
#[derive(Debug, Serialize)]
pub struct ExternalStorageResponse {
    pub storage: StorageStats,
    pub photos: ExternalPhotoStorageStats,
    pub database: ExternalDatabaseSize,
}

#[derive(Debug, Serialize)]
pub struct ExternalPhotoStorageStats {
    pub total_photos: i64,
    pub total_file_bytes: i64,
    pub total_thumb_bytes: i64,
    pub photos_by_media_type: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct ExternalDatabaseSize {
    pub size_bytes: u64,
    pub wal_size_bytes: u64,
}

/// Audit/security-focused response for SIEM integration.
#[derive(Debug, Serialize)]
pub struct ExternalAuditResponse {
    pub audit: AuditSummary,
    pub users: UserStats,
    pub client_logs: ClientLogSummary,
}
