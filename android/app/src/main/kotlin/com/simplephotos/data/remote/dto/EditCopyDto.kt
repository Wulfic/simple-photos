/**
 * Edit copy DTOs — non-destructive edit copies attached to a photo
 * (separate from `duplicate`, which forks a fully independent photo).
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class EditCopy(
    val id: String,
    val name: String? = null,
    @SerializedName("edit_metadata") val editMetadata: String? = null,
    @SerializedName("created_at") val createdAt: String,
)

data class EditCopyListResponse(
    val copies: List<EditCopy>,
)

data class CreateEditCopyRequest(
    val name: String? = null,
    @SerializedName("edit_metadata") val editMetadata: String,
)

data class CreateEditCopyResponse(
    val id: String,
    @SerializedName("photo_id") val photoId: String,
    val name: String? = null,
    @SerializedName("edit_metadata") val editMetadata: String? = null,
)

data class DeleteEditCopyResponse(val ok: Boolean)

// ── Render & download ───────────────────────────────────────────────────────

data class RenderPhotoRequest(
    @SerializedName("edit_metadata") val editMetadata: String? = null,
    @SerializedName("output_format") val outputFormat: String? = null,
    @SerializedName("quality") val quality: Int? = null,
)
