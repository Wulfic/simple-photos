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
#[derive(Debug, Serialize)]
pub struct DiscoveredServer {
    pub address: String,
    pub name: String,
    pub version: String,
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

/// Photo record exposed over the backup API (no user_id — all photos on the server).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackupPhotoRecord {
    pub id: String,
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
}

/// Request to change server backup mode.
#[derive(Debug, Deserialize)]
pub struct SetBackupModeRequest {
    pub mode: String,
}
