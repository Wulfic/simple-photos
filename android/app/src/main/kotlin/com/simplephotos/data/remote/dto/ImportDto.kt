/**
 * Server-side import DTOs — admin import scan + Google Photos pairing.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class ImportScanFile(
    val name: String,
    val path: String,
    val size: Long,
    @SerializedName("mime_type") val mimeType: String? = null,
    val modified: String? = null,
)

data class ImportScanResponse(
    val directory: String,
    val files: List<ImportScanFile>,
    @SerializedName("total_size") val totalSize: Long = 0,
)

data class GooglePhotosScanResponse(
    val directory: String,
    @SerializedName("media_files") val mediaFiles: Int,
    @SerializedName("sidecar_files") val sidecarFiles: Int,
    val paired: Int,
    @SerializedName("unpaired_media") val unpairedMedia: List<String>,
    @SerializedName("unpaired_sidecars") val unpairedSidecars: List<String>,
)

data class GooglePhotosImportRequest(val path: String)

data class GooglePhotosImportResponse(
    @SerializedName("photos_imported") val photosImported: Int,
    @SerializedName("metadata_imported") val metadataImported: Int,
    val errors: List<String> = emptyList(),
)
