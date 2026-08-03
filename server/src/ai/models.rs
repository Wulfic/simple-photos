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
    pub pet_detections: i64,
    pub pet_clusters: i64,
    /// True when the SCRFD or UltraFace ONNX detection model is loaded.
    pub face_model_loaded: bool,
    /// True when the MobileNetV2 ONNX classification model is loaded.
    pub object_model_loaded: bool,
    /// True when neither model is loaded AND `allow_heuristic_fallback`
    /// is false. In this state, AI processing runs but produces no
    /// detections — admins should fetch the models.
    pub degraded_mode: bool,
    /// Whether the operator has explicitly opted in to the degraded
    /// heuristic detectors.
    pub allow_heuristic_fallback: bool,
}

/// Face cluster summary for the clusters list endpoint.
///
/// `rep_bbox_*` describe the representative face's bounding box (normalised
/// 0.0–1.0) within the representative photo, so clients can crop the People-grid
/// tile to the detected face instead of showing the whole photo. They are
/// `None` when no matching detection exists (additive — old clients ignore them).
#[derive(Debug, Serialize, FromRow)]
pub struct FaceClusterSummary {
    pub id: i64,
    pub label: Option<String>,
    pub photo_count: i64,
    pub representative: Option<String>,
    pub rep_bbox_x: Option<f64>,
    pub rep_bbox_y: Option<f64>,
    pub rep_bbox_w: Option<f64>,
    pub rep_bbox_h: Option<f64>,
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

/// A single face detected within one photo, joined to its current cluster
/// label. Powers the per-photo "faces in this photo" list used by the manual
/// person-reassignment UI.
#[derive(Debug, Serialize, FromRow)]
pub struct PhotoFaceRecord {
    pub id: i64,
    pub cluster_id: Option<i64>,
    /// Label of the cluster this face currently belongs to (None = unnamed or
    /// not yet clustered).
    pub cluster_label: Option<String>,
    pub bbox_x: f64,
    pub bbox_y: f64,
    pub bbox_w: f64,
    pub bbox_h: f64,
    pub confidence: f64,
}

/// Pet cluster summary for the pet clusters list endpoint.
///
/// `rep_bbox_*` mirror [`FaceClusterSummary`] exactly — the representative
/// animal's box, normalised 0.0–1.0 against the whole photo — so the Pets grid
/// can frame its tile instead of centre-cropping it. `None` until the photo has
/// been processed by a build that stores the box, or when migration 039 could
/// not recover one (additive — old clients ignore them).
#[derive(Debug, Serialize, FromRow)]
pub struct PetClusterSummary {
    pub id: i64,
    pub label: Option<String>,
    pub species: String,
    pub photo_count: i64,
    pub representative: Option<String>,
    pub rep_bbox_x: Option<f64>,
    pub rep_bbox_y: Option<f64>,
    pub rep_bbox_w: Option<f64>,
    pub rep_bbox_h: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

/// Pet detection record returned by the photos-in-cluster endpoint.
#[derive(Debug, Serialize, FromRow)]
pub struct PetDetectionRecord {
    pub id: i64,
    pub photo_id: String,
    pub cluster_id: Option<i64>,
    pub species: String,
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

/// Request body for manually reassigning a single face detection to a person
/// (cluster) — the correction path when the AI clustered a face wrong.
#[derive(Debug, Deserialize)]
pub struct AssignFaceRequest {
    /// The face detection to move.
    pub detection_id: i64,
    /// The target face cluster (person) to move it into.
    pub cluster_id: i64,
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
