//! Request/response DTOs and database row models for the backup subsystem.

use serde::{Deserialize, Serialize};

/// Build a full URL from a backup server address and a path.
///
/// If `address` already contains a scheme (`http://` or `https://`), it is
/// preserved.  Otherwise `http://` is prepended (backward-compatible default).
/// The returned URL has the form `<scheme>://<address><path>`.
pub fn resolve_backup_url(address: &str, path: &str) -> String {
    let trimmed = address.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        format!("{trimmed}{path}")
    } else {
        format!("http://{trimmed}{path}")
    }
}

/// A configured backup server.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BackupServer {
    pub id: String,
    pub name: String,
    pub address: String,
    pub sync_frequency_hours: i64,
    pub last_sync_at: Option<String>,
    pub last_sync_status: String,
    pub last_sync_error: Option<String>,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct BackupServerListResponse {
    pub servers: Vec<BackupServer>,
}

#[derive(Debug, Deserialize)]
pub struct AddBackupServerRequest {
    pub name: String,
    pub address: String,
    pub api_key: Option<String>,
    pub sync_frequency_hours: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBackupServerRequest {
    pub name: Option<String>,
    pub address: Option<String>,
    pub api_key: Option<String>,
    pub sync_frequency_hours: Option<i64>,
    pub enabled: Option<bool>,
}

/// Discovered server on the local network.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredServer {
    pub address: String,
    pub name: String,
    pub version: String,
    /// Operating mode: `"primary"` or `"backup"`.
    /// `None` when the server responded via `/health` only (mode unknown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// API key for backup-mode servers discovered on localhost via /api/discover/info.
    /// `None` for LAN-discovered servers or servers not in backup mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DiscoverResponse {
    pub servers: Vec<DiscoveredServer>,
}

/// Status info returned from a backup server's health endpoint.
#[derive(Debug, Deserialize, Serialize)]
pub struct BackupServerStatus {
    pub reachable: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

/// Sync log entry.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SyncLogEntry {
    pub id: String,
    pub server_id: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub photos_synced: i64,
    pub bytes_synced: i64,
    pub error: Option<String>,
}

/// Photo record exposed over the backup API.
/// Includes `user_id` so recovery can preserve per-user ownership,
/// and all metadata columns for faithful restoration.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackupPhotoRecord {
    pub id: String,
    pub user_id: String,
    pub filename: String,
    pub file_path: String,
    pub mime_type: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub thumb_path: Option<String>,
    pub created_at: String,
    pub is_favorite: bool,
    pub camera_model: Option<String>,
    pub photo_hash: Option<String>,
    pub crop_metadata: Option<String>,
    /// The only useful part of a row with `file_path = ''` (an Android upload or
    /// an encrypted Save Copy): its bytes live in this blob. `default` so recovery
    /// still parses the list from a backup older than F9.
    #[serde(default)]
    pub encrypted_blob_id: Option<String>,
    #[serde(default)]
    pub encrypted_thumb_blob_id: Option<String>,
}

// ── Row-only transfer (F9) ───────────────────────────────────────────────────
//
// A row with `file_path = ''` has no plaintext file to send through
// `/backup/receive`: its bytes travel as blobs (sync phase 3). These DTOs carry
// just the row. They deliberately have NO path field — the backup stores `''`
// and never takes a path from this payload.

/// Most rows one `receive-*-rows` request may carry.
pub const MAX_ROWS_PER_BATCH: usize = 200;

/// A `photos` row with no plaintext file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncryptedPhotoRow {
    pub id: String,
    pub user_id: String,
    pub filename: String,
    pub mime_type: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub camera_model: Option<String>,
    pub is_favorite: bool,
    pub photo_hash: Option<String>,
    pub crop_metadata: Option<String>,
    pub created_at: String,
    pub encrypted_blob_id: Option<String>,
    pub encrypted_thumb_blob_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A `trash_items` row with no plaintext file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncryptedTrashRow {
    pub id: String,
    /// The trashed photo's `photos.id` — the backup removes its live row by it.
    pub photo_id: String,
    pub user_id: String,
    pub filename: String,
    pub mime_type: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
    pub duration_secs: Option<f64>,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub camera_model: Option<String>,
    pub is_favorite: bool,
    pub photo_hash: Option<String>,
    pub crop_metadata: Option<String>,
    pub deleted_at: String,
    pub expires_at: String,
    pub encrypted_blob_id: Option<String>,
    pub encrypted_thumb_blob_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RowBatch<T> {
    pub rows: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RowFailure {
    pub id: String,
    pub error: String,
}

/// Per-row outcome of a `receive-*-rows` request: one bad row fails alone.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RowBatchResult {
    pub accepted: Vec<String>,
    pub failed: Vec<RowFailure>,
}

/// Recovery progress response.
#[derive(Debug, Serialize)]
pub struct RecoveryResponse {
    pub message: String,
    pub recovery_id: String,
}

/// Response for backup mode status.
#[derive(Debug, Serialize)]
pub struct BackupModeResponse {
    pub mode: String,
    pub server_ip: String,
    pub server_address: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// The URL of the primary server this backup is paired with (only set in backup mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_server_url: Option<String>,
}

/// Request to change server backup mode.
#[derive(Debug, Deserialize)]
pub struct SetBackupModeRequest {
    pub mode: String,
}

/// Health/diagnostics snapshot pushed by a backup server to its primary.
///
/// Collected every 15 minutes by the backup and POSTed to
/// `POST /api/backup/report` on the primary server.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupDiagnosticsReport {
    /// Server version string
    pub version: String,
    /// Seconds since the backup server process started
    pub uptime_seconds: u64,
    /// Resident memory in bytes (Linux only; 0 on other platforms)
    pub memory_rss_bytes: u64,
    /// Accumulated CPU time in seconds (user + system, Linux only)
    pub cpu_seconds: f64,
    /// Total photos stored on this backup server
    pub total_photos: i64,
    /// Percentage of the storage disk that is used (0–100)
    pub disk_used_percent: f64,
    /// Database file size in bytes
    pub db_size_bytes: u64,
    /// RFC-3339 timestamp at which this report was collected on the backup
    pub collected_at: String,
}

/// A single audit log entry forwarded from a backup server to the primary.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ForwardedAuditLog {
    pub id: String,
    pub event_type: String,
    pub user_id: Option<String>,
    pub ip_address: String,
    pub user_agent: String,
    pub details: String,
    pub created_at: String,
}

/// Batch of forwarded audit log entries.
#[derive(Debug, Deserialize)]
pub struct ForwardLogsRequest {
    pub logs: Vec<ForwardedAuditLog>,
}

/// Response for log forwarding.
#[derive(Debug, Serialize)]
pub struct ForwardLogsResponse {
    pub accepted: usize,
}
