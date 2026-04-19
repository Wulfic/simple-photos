//! AI data transfer objects and database models.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// A face detection bounding box (normalised 0.0–1.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Raw face detection result from the ML model (not stored directly).
#[derive(Debug, Clone)]
pub struct FaceDetection {
    pub bbox: BoundingBox,
    pub confidence: f32,
    /// Embedding vector (populated after embedding extraction).
    #[allow(dead_code)] // Populated in pipeline, stored to DB
    pub embedding: Vec<f32>,
}

/// Object detection result from the ML model (not stored directly).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectDetection {
    pub class_name: String,
    pub confidence: f32,
    pub bbox: BoundingBox,
}

// ── API response types ───────────────────────────────────────────────

/// AI status response for GET /api/ai/status.
#[derive(Debug, Serialize)]
pub struct AiStatusResponse {
    pub enabled: bool,
    pub gpu_available: bool,
    pub photos_processed: i64,
    pub photos_pending: i64,
    pub face_detections: i64,
    pub face_clusters: i64,
    pub object_detections: i64,
}

/// Face cluster summary for the clusters list endpoint.
#[derive(Debug, Serialize, FromRow)]
pub struct FaceClusterSummary {
    pub id: i64,
    pub label: Option<String>,
    pub photo_count: i64,
    pub representative: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Face detection record returned by the photos-in-cluster endpoint.
#[derive(Debug, Serialize, FromRow)]
pub struct FaceDetectionRecord {
    pub id: i64,
    pub photo_id: String,
    pub cluster_id: Option<i64>,
    pub bbox_x: f64,
    pub bbox_y: f64,
    pub bbox_w: f64,
    pub bbox_h: f64,
    pub confidence: f64,
    pub created_at: String,
}

/// Object detection record from the database.
#[derive(Debug, Serialize, FromRow)]
pub struct ObjectDetectionRecord {
    pub id: i64,
    pub photo_id: String,
    pub class_name: String,
    pub confidence: f64,
    pub bbox_x: f64,
    pub bbox_y: f64,
    pub bbox_w: f64,
    pub bbox_h: f64,
    pub created_at: String,
}

/// Object class summary — unique object type with count.
#[derive(Debug, Serialize, FromRow)]
pub struct ObjectClassSummary {
    pub class_name: String,
    pub photo_count: i64,
    pub avg_confidence: f64,
}

// ── Request types ────────────────────────────────────────────────────

/// Request body for renaming a face cluster.
#[derive(Debug, Deserialize)]
pub struct RenameFaceRequest {
    pub name: String,
}

/// Request body for merging face clusters.
#[derive(Debug, Deserialize)]
pub struct MergeFacesRequest {
    pub cluster_ids: Vec<i64>,
}

/// Request body for splitting faces out of a cluster.
#[derive(Debug, Deserialize)]
pub struct SplitFacesRequest {
    pub detection_ids: Vec<i64>,
}

/// Request body for toggling AI processing.
#[derive(Debug, Deserialize)]
pub struct AiToggleRequest {
    pub enabled: bool,
}

/// Request body for triggering reprocessing.
#[derive(Debug, Deserialize)]
pub struct AiReprocessRequest {
    /// If set, only reprocess these specific photo IDs.
    pub photo_ids: Option<Vec<String>>,
}
