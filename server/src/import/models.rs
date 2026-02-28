use serde::{Deserialize, Serialize};

// ── Google Photos Takeout JSON schema ────────────────────────────────────────

/// Top-level structure of a Google Photos metadata JSON sidecar file.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GooglePhotosMetadata {
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub image_views: Option<String>,
    pub creation_time: Option<GoogleTimestamp>,
    pub photo_taken_time: Option<GoogleTimestamp>,
    pub geo_data: Option<GoogleGeoData>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub google_photos_origin: Option<serde_json::Value>,
}

/// Google Photos timestamp with Unix epoch and formatted string.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GoogleTimestamp {
    pub timestamp: Option<String>,
    pub formatted: Option<String>,
}

/// Google Photos geo data with lat/lng/alt.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GoogleGeoData {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub latitude_span: Option<f64>,
    pub longitude_span: Option<f64>,
}

// ── Internal metadata model ──────────────────────────────────────────────────

/// Normalised metadata extracted from any source (Google Photos, EXIF, manual).
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct PhotoMetadataRecord {
    pub id: String,
    pub user_id: String,
    pub photo_id: Option<String>,
    pub blob_id: Option<String>,
    pub source: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub taken_at: Option<String>,
    pub created_at_src: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub image_views: Option<i64>,
    pub original_url: Option<String>,
    pub storage_path: Option<String>,
    pub is_encrypted: bool,
    pub imported_at: String,
}

/// Request to import a Google Photos metadata JSON file.
#[derive(Debug, Deserialize)]
pub struct GooglePhotosImportRequest {
    /// The Google Photos JSON metadata content (parsed).
    pub metadata: GooglePhotosMetadata,
    /// Optional: associate with an existing photo (plain mode).
    pub photo_id: Option<String>,
    /// Optional: associate with an existing blob (encrypted mode).
    pub blob_id: Option<String>,
}

/// Batch import request for Google Photos Takeout.
#[derive(Debug, Deserialize)]
pub struct GooglePhotosBatchImportRequest {
    pub entries: Vec<GooglePhotosImportRequest>,
}

/// Response for a single import.
#[derive(Debug, Serialize)]
pub struct ImportMetadataResponse {
    pub metadata_id: String,
    pub storage_path: Option<String>,
    pub is_encrypted: bool,
}

/// Response for batch import.
#[derive(Debug, Serialize)]
pub struct BatchImportResponse {
    pub imported: usize,
    pub failed: usize,
    pub results: Vec<ImportMetadataResultEntry>,
}

#[derive(Debug, Serialize)]
pub struct ImportMetadataResultEntry {
    pub index: usize,
    pub metadata_id: Option<String>,
    pub error: Option<String>,
}

/// Response for listing metadata.
#[derive(Debug, Serialize)]
pub struct MetadataListResponse {
    pub metadata: Vec<PhotoMetadataRecord>,
    pub next_cursor: Option<String>,
}
