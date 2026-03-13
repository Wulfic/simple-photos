//! DTOs for the tagging and search endpoints.

use serde::{Deserialize, Serialize};

/// Response for `GET /api/tags` — all distinct tags for the authenticated user.
#[derive(Debug, Serialize, Deserialize)]
pub struct TagListResponse {
    pub tags: Vec<String>,
}

/// Response for `GET /api/photos/:id/tags`.
#[derive(Debug, Serialize, Deserialize)]
pub struct PhotoTagsResponse {
    pub photo_id: String,
    pub tags: Vec<String>,
}

/// Request body for `POST /api/photos/:id/tags`.
#[derive(Debug, Deserialize)]
pub struct AddTagRequest {
    pub tag: String,
}

/// Request body for `DELETE /api/photos/:id/tags`.
#[derive(Debug, Deserialize)]
pub struct RemoveTagRequest {
    pub tag: String,
}

/// Query parameters for `GET /api/search`.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Search term — matched against tag names and filenames.
    pub q: String,
    /// Maximum results to return (default 100, capped at 500 by handler).
    pub limit: Option<i64>,
}

/// A single search result (photo or video matching the query).
#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub id: String,
    pub filename: String,
    pub media_type: String,
    pub mime_type: String,
    pub thumb_path: Option<String>,
    pub created_at: String,
    pub taken_at: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub tags: Vec<String>,
}

/// Wrapper for search results.
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}
