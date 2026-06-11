/**
 * AI feature DTOs — face clusters, object classification, pet clusters,
 * AI status / toggle / reprocess controls.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class AiStatusResponse(
    val enabled: Boolean,
    @SerializedName("models_loaded") val modelsLoaded: Boolean = false,
    @SerializedName("processing_active") val processingActive: Boolean = false,
    @SerializedName("queue_size") val queueSize: Int = 0,
    @SerializedName("processed_count") val processedCount: Int = 0,
    @SerializedName("total_photos") val totalPhotos: Int = 0,
    val message: String? = null,
)

data class AiToggleRequest(val enabled: Boolean)
data class AiToggleResponse(val enabled: Boolean, val message: String? = null)

data class AiReprocessRequest(
    @SerializedName("scope") val scope: String? = null,
)
data class AiReprocessResponse(
    val message: String,
    @SerializedName("queued") val queued: Int = 0,
)

// ── Face clusters ───────────────────────────────────────────────────────────

data class FaceCluster(
    val id: Long,
    val label: String? = null,
    @SerializedName("photo_count") val photoCount: Int,
    val representative: String? = null,
    @SerializedName("created_at") val createdAt: String? = null,
    @SerializedName("updated_at") val updatedAt: String? = null,
)

data class FaceClusterMergeRequest(
    @SerializedName("source_cluster_id") val sourceClusterId: String,
    @SerializedName("target_cluster_id") val targetClusterId: String,
)

data class FaceClusterSplitRequest(
    @SerializedName("cluster_id") val clusterId: String,
    @SerializedName("face_ids") val faceIds: List<String>,
)

data class FaceClusterRenameRequest(val name: String)

data class FaceClusterPhotoEntry(
    val id: String,
    @SerializedName("photo_id") val photoId: String,
    @SerializedName("blob_id") val blobId: String? = null,
    @SerializedName("face_id") val faceId: String? = null,
    @SerializedName("confidence") val confidence: Double? = null,
)

data class FaceClusterPhotosResponse(
    val photos: List<FaceClusterPhotoEntry>,
)

// ── Object classes ──────────────────────────────────────────────────────────

data class ObjectClass(
    @SerializedName("class_name") val className: String,
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("preview_photo_id") val previewPhotoId: String? = null,
)

data class ObjectClassListResponse(
    val classes: List<ObjectClass>,
)

data class ObjectClassPhotoEntry(
    @SerializedName("photo_id") val photoId: String,
    @SerializedName("blob_id") val blobId: String? = null,
    val confidence: Double? = null,
)

data class ObjectClassPhotosResponse(
    val photos: List<ObjectClassPhotoEntry>,
)

// ── Pet clusters ────────────────────────────────────────────────────────────

data class PetCluster(
    val id: Long,
    val label: String? = null,
    val species: String,
    @SerializedName("photo_count") val photoCount: Int,
    val representative: String? = null,
    @SerializedName("created_at") val createdAt: String? = null,
    @SerializedName("updated_at") val updatedAt: String? = null,
)

data class PetClusterMergeRequest(
    @SerializedName("source_cluster_id") val sourceClusterId: String,
    @SerializedName("target_cluster_id") val targetClusterId: String,
)

data class PetClusterRenameRequest(val name: String)

data class PetClusterPhotoEntry(
    @SerializedName("photo_id") val photoId: String,
    @SerializedName("blob_id") val blobId: String? = null,
    val confidence: Double? = null,
)

data class PetClusterPhotosResponse(
    val photos: List<PetClusterPhotoEntry>,
)
