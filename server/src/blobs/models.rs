use serde::Serialize;

/// A single blob record as returned by the list endpoint.
/// Does not include `storage_path` (server-internal) or user ownership info.
#[derive(Debug, sqlx::FromRow, Serialize)]
pub struct BlobRecord {
    pub id: String,
    pub blob_type: String,
    pub size_bytes: i64,
    pub client_hash: Option<String>,
    pub upload_time: String,
    pub content_hash: Option<String>,
}

/// Paginated blob list response.
#[derive(Debug, Serialize)]
pub struct BlobListResponse {
    pub blobs: Vec<BlobRecord>,
    /// Upload time of the last item — pass as `after` for the next page.
    pub next_cursor: Option<String>,
}

/// Response body returned after a successful blob upload.
#[derive(Debug, Serialize)]
pub struct BlobUploadResponse {
    pub blob_id: String,
    pub upload_time: String,
    pub size: i64,
}
