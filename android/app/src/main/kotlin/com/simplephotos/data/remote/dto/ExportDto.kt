/**
 * Export pipeline DTOs — start an export, poll status, list completed
 * archives, and download a finished file.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class ExportStartRequest(
    @SerializedName("scope") val scope: String? = null,
    @SerializedName("photo_ids") val photoIds: List<String>? = null,
    @SerializedName("album_id") val albumId: String? = null,
    @SerializedName("include_metadata") val includeMetadata: Boolean = true,
    @SerializedName("decrypt") val decrypt: Boolean = true,
    @SerializedName("strip_geo") val stripGeo: Boolean = false,
)

data class ExportStartResponse(
    @SerializedName("export_id") val exportId: String,
    val message: String? = null,
)

data class ExportStatusResponse(
    @SerializedName("export_id") val exportId: String? = null,
    val state: String,
    val progress: Double = 0.0,
    @SerializedName("processed_count") val processedCount: Int = 0,
    @SerializedName("total_count") val totalCount: Int = 0,
    @SerializedName("error") val error: String? = null,
    val message: String? = null,
)

data class ExportFile(
    val id: String,
    val filename: String,
    @SerializedName("size_bytes") val sizeBytes: Long,
    @SerializedName("created_at") val createdAt: String,
    @SerializedName("photo_count") val photoCount: Int = 0,
)

data class ExportFileListResponse(
    val files: List<ExportFile>,
)
