//! Request/response DTOs and database row models for the backup subsystem.

use serde::{Deserialize, Serialize};

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
