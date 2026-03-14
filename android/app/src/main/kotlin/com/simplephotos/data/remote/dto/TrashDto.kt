/**
 * Trash DTOs — soft-deleted photo list items with remaining days until
 * automatic permanent deletion (30-day retention).
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class TrashItemDto(
    val id: String,
    @SerializedName("photo_id") val photoId: String,
    val filename: String,
    @SerializedName("file_path") val filePath: String,
    @SerializedName("mime_type") val mimeType: String,
    @SerializedName("media_type") val mediaType: String,
    @SerializedName("size_bytes") val sizeBytes: Long,
    val width: Long,
    val height: Long,
    @SerializedName("duration_secs") val durationSecs: Double? = null,
    @SerializedName("taken_at") val takenAt: String? = null,
    val latitude: Double? = null,
    val longitude: Double? = null,
    @SerializedName("thumb_path") val thumbPath: String? = null,
    @SerializedName("deleted_at") val deletedAt: String,
    @SerializedName("expires_at") val expiresAt: String,
)

data class TrashListResponse(
    val items: List<TrashItemDto>,
    @SerializedName("next_cursor") val nextCursor: String?,
)
