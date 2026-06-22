/**
 * DTOs for the photo-metadata EDIT API — the Android mirror of the web
 * client `web/src/api/metadata.ts` (server routes in `metadata_edit.rs`):
 *   GET   /api/photos/{id}/metadata/full        → [FullMetadataResponse]
 *   PATCH /api/photos/{id}/metadata             → [MetadataUpdateResponse]  (send only changed fields)
 *   POST  /api/photos/{id}/metadata/write-exif  → [WriteExifResponse]
 *
 * (Distinct from the Google-Photos sidecar DTOs in MetadataDto.kt.)
 *
 * [MetadataUpdateRequest] is all-nullable and built with ONLY the changed
 * fields populated; the Retrofit Gson converter (`GsonConverterFactory.create()`,
 * no `serializeNulls()`) omits nulls on the wire, so the "only-changed-fields"
 * PATCH semantics are automatic — matching the web client's diff-and-send.
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

data class MetadataUpdateRequest(
    @SerializedName("filename") val filename: String? = null,
    @SerializedName("taken_at") val takenAt: String? = null,
    @SerializedName("latitude") val latitude: Double? = null,
    @SerializedName("longitude") val longitude: Double? = null,
    @SerializedName("camera_model") val cameraModel: String? = null,
    /** Manual subtype correction: "none" | "panorama" | "equirectangular". */
    @SerializedName("photo_subtype") val photoSubtype: String? = null,
    @SerializedName("clear_gps") val clearGps: Boolean? = null,
    @SerializedName("camera_make") val cameraMake: String? = null,
    @SerializedName("lens_model") val lensModel: String? = null,
    @SerializedName("iso_speed") val isoSpeed: Int? = null,
    @SerializedName("f_number") val fNumber: Double? = null,
    @SerializedName("exposure_time") val exposureTime: String? = null,
    @SerializedName("focal_length") val focalLength: Double? = null,
    @SerializedName("flash") val flash: String? = null,
    @SerializedName("white_balance") val whiteBalance: String? = null,
    @SerializedName("exposure_program") val exposureProgram: String? = null,
    @SerializedName("metering_mode") val meteringMode: String? = null,
    @SerializedName("orientation") val orientation: Int? = null,
    @SerializedName("software") val software: String? = null,
    @SerializedName("artist") val artist: String? = null,
    @SerializedName("copyright") val copyright: String? = null,
    @SerializedName("description") val description: String? = null,
    @SerializedName("user_comment") val userComment: String? = null,
    @SerializedName("color_space") val colorSpace: String? = null,
    @SerializedName("exposure_bias") val exposureBias: Double? = null,
    @SerializedName("scene_type") val sceneType: String? = null,
    @SerializedName("digital_zoom") val digitalZoom: Double? = null,
    @SerializedName("exif_overrides") val exifOverrides: Map<String, String>? = null,
)

data class MetadataUpdateResponse(
    @SerializedName("status") val status: String,
    @SerializedName("updated_fields") val updatedFields: List<String> = emptyList(),
)

data class FullMetadataResponse(
    @SerializedName("id") val id: String,
    @SerializedName("filename") val filename: String,
    @SerializedName("mime_type") val mimeType: String,
    @SerializedName("media_type") val mediaType: String,
    @SerializedName("width") val width: Int,
    @SerializedName("height") val height: Int,
    @SerializedName("size_bytes") val sizeBytes: Long,
    @SerializedName("taken_at") val takenAt: String? = null,
    @SerializedName("latitude") val latitude: Double? = null,
    @SerializedName("longitude") val longitude: Double? = null,
    @SerializedName("camera_model") val cameraModel: String? = null,
    @SerializedName("photo_hash") val photoHash: String? = null,
    @SerializedName("photo_subtype") val photoSubtype: String? = null,
    @SerializedName("geo_city") val geoCity: String? = null,
    @SerializedName("geo_state") val geoState: String? = null,
    @SerializedName("geo_country") val geoCountry: String? = null,
    @SerializedName("geo_country_code") val geoCountryCode: String? = null,
    @SerializedName("photo_year") val photoYear: Int? = null,
    @SerializedName("photo_month") val photoMonth: Int? = null,
    @SerializedName("created_at") val createdAt: String,
    @SerializedName("camera_make") val cameraMake: String? = null,
    @SerializedName("lens_model") val lensModel: String? = null,
    @SerializedName("iso_speed") val isoSpeed: Int? = null,
    @SerializedName("f_number") val fNumber: Double? = null,
    @SerializedName("exposure_time") val exposureTime: String? = null,
    @SerializedName("focal_length") val focalLength: Double? = null,
    @SerializedName("flash") val flash: String? = null,
    @SerializedName("white_balance") val whiteBalance: String? = null,
    @SerializedName("exposure_program") val exposureProgram: String? = null,
    @SerializedName("metering_mode") val meteringMode: String? = null,
    @SerializedName("orientation") val orientation: Int? = null,
    @SerializedName("software") val software: String? = null,
    @SerializedName("artist") val artist: String? = null,
    @SerializedName("copyright") val copyright: String? = null,
    @SerializedName("description") val description: String? = null,
    @SerializedName("user_comment") val userComment: String? = null,
    @SerializedName("color_space") val colorSpace: String? = null,
    @SerializedName("exposure_bias") val exposureBias: Double? = null,
    @SerializedName("scene_type") val sceneType: String? = null,
    @SerializedName("digital_zoom") val digitalZoom: Double? = null,
    @SerializedName("exif_overrides") val exifOverrides: Map<String, String>? = null,
    @SerializedName("exif_tags") val exifTags: Map<String, String>? = null,
)

data class WriteExifResponse(
    @SerializedName("status") val status: String,
    @SerializedName("new_photo_hash") val newPhotoHash: String? = null,
)
