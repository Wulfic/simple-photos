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
    @SerializedName("photo_hash") val photoHash: String? = null,
    // Special-photo classification detected client-side from the plaintext
    // bytes (the server only ever sees the encrypted blob and can't scan XMP).
    @SerializedName("photo_subtype") val photoSubtype: String? = null,
    @SerializedName("burst_id") val burstId: String? = null,
    // Camera make/model from EXIF. Required server-side for timestamp-based
    // burst grouping of cameras (e.g. Samsung) that don't write an XMP BurstID.
    @SerializedName("camera_model") val cameraModel: String? = null
)

/** Body of `POST /api/photos/lookup-hashes` (≤ 500 hashes). */
data class LookupHashesRequest(val hashes: List<String>)

/** A miss is absent from [matches]. Servers before `87927fd` answer 404. */
data class LookupHashesResponse(val matches: Map<String, HashMatchDto>? = null)

/** The server photo already holding some content, and the blobs it serves. */
data class HashMatchDto(
    @SerializedName("photo_id") val photoId: String? = null,
    @SerializedName("encrypted_blob_id") val encryptedBlobId: String? = null,
    @SerializedName("encrypted_thumb_blob_id") val encryptedThumbBlobId: String? = null,
)

data class RegisterEncryptedPhotoResponse(
    @SerializedName("photo_id") val photoId: String,
    val duplicate: Boolean? = null,
    /**
     * On a duplicate: the blobs the EXISTING photo serves (servers from `9c3a7a0`). Record
     * these, not the ones just uploaded — those are referenced by nothing
     * server-side. Null from older servers.
     */
    @SerializedName("encrypted_blob_id") val encryptedBlobId: String? = null,
    @SerializedName("encrypted_thumb_blob_id") val encryptedThumbBlobId: String? = null,
)
