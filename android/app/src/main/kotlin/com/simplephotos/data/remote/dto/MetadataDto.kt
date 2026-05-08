/**
 * Photo metadata sidecar DTOs — Google Photos JSON imports and
 * per-photo metadata records.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

/**
 * Mirrors Google Photos Takeout sidecar JSON. Fields are optional because
 * not every export populates every key. The server stores the raw payload
 * via `JsonObject` — we use a `Map<String, Any?>` here for simplicity.
 */
data class GooglePhotosMetadata(
    val title: String? = null,
    val description: String? = null,
    @SerializedName("imageViews") val imageViews: String? = null,
    @SerializedName("creationTime") val creationTime: GoogleTimeStamp? = null,
    @SerializedName("photoTakenTime") val photoTakenTime: GoogleTimeStamp? = null,
    @SerializedName("geoData") val geoData: GoogleGeoData? = null,
    @SerializedName("geoDataExif") val geoDataExif: GoogleGeoData? = null,
    val people: List<GooglePerson>? = null,
    val url: String? = null,
    val googlePhotosOrigin: Map<String, Any?>? = null,
)

data class GoogleTimeStamp(
    val timestamp: String? = null,
    val formatted: String? = null,
)

data class GoogleGeoData(
    val latitude: Double? = null,
    val longitude: Double? = null,
    val altitude: Double? = null,
    val latitudeSpan: Double? = null,
    val longitudeSpan: Double? = null,
)

data class GooglePerson(val name: String)

data class ImportMetadataRequest(
    val metadata: GooglePhotosMetadata,
    @SerializedName("photo_id") val photoId: String? = null,
    @SerializedName("blob_id") val blobId: String? = null,
)

data class ImportMetadataResponse(
    @SerializedName("metadata_id") val metadataId: String,
    @SerializedName("storage_path") val storagePath: String? = null,
)

data class ImportMetadataBatchEntry(
    val metadata: GooglePhotosMetadata,
    @SerializedName("photo_id") val photoId: String? = null,
    @SerializedName("blob_id") val blobId: String? = null,
)

data class ImportMetadataBatchRequest(
    val entries: List<ImportMetadataBatchEntry>,
)

data class ImportMetadataBatchResultEntry(
    val index: Int,
    @SerializedName("metadata_id") val metadataId: String? = null,
    val error: String? = null,
)

data class ImportMetadataBatchResponse(
    val imported: Int,
    val failed: Int,
    val results: List<ImportMetadataBatchResultEntry>,
)

data class PhotoMetadataRecord(
    val id: String,
    @SerializedName("photo_id") val photoId: String? = null,
    @SerializedName("blob_id") val blobId: String? = null,
    val source: String? = null,
    @SerializedName("created_at") val createdAt: String,
    val metadata: GooglePhotosMetadata? = null,
    @SerializedName("storage_path") val storagePath: String? = null,
)

data class PhotoMetadataListResponse(
    val metadata: List<PhotoMetadataRecord>,
    @SerializedName("next_cursor") val nextCursor: String? = null,
)
