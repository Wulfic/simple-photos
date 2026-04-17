/**
 * Blob storage DTOs — upload metadata, blob list responses, and encrypted
 * blob identifiers used in encrypted mode.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class BlobUploadResponse(
    @SerializedName("blob_id") val blobId: String,
    @SerializedName("upload_time") val uploadTime: String,
    val size: Long
)

data class BlobListResponse(
    val blobs: List<BlobRecord>,
    @SerializedName("next_cursor") val nextCursor: String?
)

data class BlobRecord(
    val id: String,
    @SerializedName("blob_type") val blobType: String,
    @SerializedName("size_bytes") val sizeBytes: Long,
    @SerializedName("client_hash") val clientHash: String?,
    @SerializedName("upload_time") val uploadTime: String,
    @SerializedName("content_hash") val contentHash: String? = null
)

data class RegisterEncryptedPhotoRequest(
    val filename: String,
    @SerializedName("mime_type") val mimeType: String,
    @SerializedName("media_type") val mediaType: String,
    val width: Int,
    val height: Int,
    @SerializedName("duration_secs") val durationSecs: Double? = null,
    @SerializedName("taken_at") val takenAt: String? = null,
    val latitude: Double? = null,
    val longitude: Double? = null,
    @SerializedName("encrypted_blob_id") val encryptedBlobId: String,
    @SerializedName("encrypted_thumb_blob_id") val encryptedThumbBlobId: String? = null,
    @SerializedName("photo_hash") val photoHash: String? = null
)

data class RegisterEncryptedPhotoResponse(
    @SerializedName("photo_id") val photoId: String,
    val duplicate: Boolean? = null
)
