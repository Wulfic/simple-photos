use serde::{Deserialize, Serialize};

/// A photo in the trash bin, pending permanent deletion.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TrashItem {
    pub id: String,
    pub photo_id: String,
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
    pub deleted_at: String,
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
pub struct TrashListResponse {
    pub items: Vec<TrashItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TrashListQuery {
    pub after: Option<String>,
    pub limit: Option<i64>,
}
