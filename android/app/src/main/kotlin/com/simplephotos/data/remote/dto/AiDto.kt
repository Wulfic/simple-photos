/**
 * AI feature DTOs — face clusters, object classification, pet clusters,
 * AI status / toggle / reprocess controls.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// Mirrors server `AiStatusResponse` (server/src/ai/models.rs).
data class AiStatusResponse(
    val enabled: Boolean,
    @SerializedName("gpu_available") val gpuAvailable: Boolean = false,
    @SerializedName("photos_processed") val photosProcessed: Int = 0,
    @SerializedName("photos_pending") val photosPending: Int = 0,
    @SerializedName("face_detections") val faceDetections: Int = 0,
    @SerializedName("face_clusters") val faceClusters: Int = 0,
    @SerializedName("object_detections") val objectDetections: Int = 0,
    @SerializedName("pet_detections") val petDetections: Int = 0,
    @SerializedName("pet_clusters") val petClusters: Int = 0,
    @SerializedName("face_model_loaded") val faceModelLoaded: Boolean = false,
    @SerializedName("object_model_loaded") val objectModelLoaded: Boolean = false,
    @SerializedName("degraded_mode") val degradedMode: Boolean = false,
    @SerializedName("allow_heuristic_fallback") val allowHeuristicFallback: Boolean = false,
) {
    /** Convenience: total = processed + pending. */
    val totalPhotos: Int get() = photosProcessed + photosPending

    /** Convenience: at least one ONNX model is loaded. */
    val modelsLoaded: Boolean get() = faceModelLoaded || objectModelLoaded
}

data class AiToggleRequest(val enabled: Boolean)

// Server `AiReprocessRequest` — optional list of specific photo IDs.
data class AiReprocessRequest(
    @SerializedName("photo_ids") val photoIds: List<String>? = null,
)

// Server reprocess returns `{ cleared, message }`.
data class AiReprocessResponse(
    val cleared: Int = 0,
    val message: String? = null,
)

// ── Face clusters ───────────────────────────────────────────────────────────

// Mirrors server `FaceClusterSummary` (bare array from GET /api/ai/faces).
data class FaceCluster(
    val id: Long,
<<<<<<< HEAD
    @SerializedName("label") val name: String? = null,
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("representative") val previewPhotoId: String? = null,
=======
    val label: String? = null,
    @SerializedName("photo_count") val photoCount: Int,
    val representative: String? = null,
>>>>>>> 3315990a7559810e2a25987163dbb852dc81a4cf
    @SerializedName("created_at") val createdAt: String? = null,
    @SerializedName("updated_at") val updatedAt: String? = null,
)

<<<<<<< HEAD
// Server `MergeFacesRequest { cluster_ids: [i64] }`.
=======
>>>>>>> 3315990a7559810e2a25987163dbb852dc81a4cf
data class FaceClusterMergeRequest(
    @SerializedName("cluster_ids") val clusterIds: List<Long>,
)

// Server `SplitFacesRequest { detection_ids: [i64] }`.
data class FaceClusterSplitRequest(
    @SerializedName("detection_ids") val detectionIds: List<Long>,
)

data class FaceClusterRenameRequest(val name: String)

// Mirrors server `FaceDetectionRecord` (bare array from .../photos).
data class FaceClusterPhotoEntry(
    val id: Long = 0,
    @SerializedName("photo_id") val photoId: String,
    @SerializedName("cluster_id") val clusterId: Long? = null,
    @SerializedName("bbox_x") val bboxX: Double = 0.0,
    @SerializedName("bbox_y") val bboxY: Double = 0.0,
    @SerializedName("bbox_w") val bboxW: Double = 0.0,
    @SerializedName("bbox_h") val bboxH: Double = 0.0,
    val confidence: Double? = null,
    @SerializedName("created_at") val createdAt: String? = null,
)

// ── Object classes ──────────────────────────────────────────────────────────

// Mirrors server `ObjectClassSummary` (bare array from GET /api/ai/objects).
data class ObjectClass(
    @SerializedName("class_name") val className: String,
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("avg_confidence") val avgConfidence: Double = 0.0,
)

// Mirrors server `ObjectDetectionRecord` (bare array from .../photos).
data class ObjectClassPhotoEntry(
    val id: Long = 0,
    @SerializedName("photo_id") val photoId: String,
    @SerializedName("class_name") val className: String? = null,
    val confidence: Double? = null,
    @SerializedName("bbox_x") val bboxX: Double = 0.0,
    @SerializedName("bbox_y") val bboxY: Double = 0.0,
    @SerializedName("bbox_w") val bboxW: Double = 0.0,
    @SerializedName("bbox_h") val bboxH: Double = 0.0,
    @SerializedName("created_at") val createdAt: String? = null,
)

// ── Pet clusters ────────────────────────────────────────────────────────────

// Mirrors server `PetClusterSummary` (bare array from GET /api/ai/pets).
data class PetCluster(
    val id: Long,
<<<<<<< HEAD
    @SerializedName("label") val name: String? = null,
    @SerializedName("species") val species: String? = null,
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("representative") val previewPhotoId: String? = null,
=======
    val label: String? = null,
    val species: String,
    @SerializedName("photo_count") val photoCount: Int,
    val representative: String? = null,
>>>>>>> 3315990a7559810e2a25987163dbb852dc81a4cf
    @SerializedName("created_at") val createdAt: String? = null,
    @SerializedName("updated_at") val updatedAt: String? = null,
)

<<<<<<< HEAD
// Pet merge shares the server `MergeFacesRequest { cluster_ids: [i64] }` shape.
=======
>>>>>>> 3315990a7559810e2a25987163dbb852dc81a4cf
data class PetClusterMergeRequest(
    @SerializedName("cluster_ids") val clusterIds: List<Long>,
)

data class PetClusterRenameRequest(val name: String)

// Mirrors server `PetDetectionRecord` (bare array from .../photos).
data class PetClusterPhotoEntry(
    val id: Long = 0,
    @SerializedName("photo_id") val photoId: String,
    @SerializedName("cluster_id") val clusterId: Long? = null,
    val species: String? = null,
    val confidence: Double? = null,
    @SerializedName("created_at") val createdAt: String? = null,
)
