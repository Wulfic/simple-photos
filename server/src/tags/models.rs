use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct TagListResponse {
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PhotoTagsResponse {
    pub photo_id: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddTagRequest {
    pub tag: String,
}

#[derive(Debug, Deserialize)]
pub struct RemoveTagRequest {
    pub tag: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<i64>,
}

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

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}
