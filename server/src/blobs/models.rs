use serde::Serialize;

#[derive(Debug, sqlx::FromRow, Serialize)]
pub struct BlobRecord {
    pub id: String,
    pub blob_type: String,
    pub size_bytes: i64,
    pub client_hash: Option<String>,
    pub upload_time: String,
}

#[derive(Debug, Serialize)]
pub struct BlobListResponse {
    pub blobs: Vec<BlobRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BlobUploadResponse {
    pub blob_id: String,
    pub upload_time: String,
    pub size: i64,
}
