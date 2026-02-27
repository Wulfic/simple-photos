use serde::{Deserialize, Serialize};

/// A single log entry sent from a mobile client.
#[derive(Debug, Deserialize)]
pub struct ClientLogEntry {
    pub level: String,
    pub tag: String,
    pub message: String,
    #[serde(default)]
    pub context: Option<serde_json::Value>,
    pub client_ts: String,
}

/// Batch of log entries from a single session.
#[derive(Debug, Deserialize)]
pub struct ClientLogBatch {
    pub session_id: String,
    pub entries: Vec<ClientLogEntry>,
}

/// Single log record returned from the query endpoint.
#[derive(Debug, Serialize)]
pub struct ClientLogRecord {
    pub id: String,
    pub user_id: String,
    pub session_id: String,
    pub level: String,
    pub tag: String,
    pub message: String,
    pub context: Option<serde_json::Value>,
    pub client_ts: String,
    pub created_at: String,
}

/// Response for listing client logs.
#[derive(Debug, Serialize)]
pub struct ClientLogListResponse {
    pub logs: Vec<ClientLogRecord>,
    pub next_cursor: Option<String>,
}
