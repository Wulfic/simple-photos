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
