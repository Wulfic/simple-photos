/**
 * Geo feature DTOs — locations, countries, map, timeline, memories, trips,
 * and geo settings (incl. scrub).
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// ── Settings ────────────────────────────────────────────────────────────────

data class GeoSettings(
    @SerializedName("geo_enabled") val geoEnabled: Boolean,
    @SerializedName("reverse_geocode_enabled") val reverseGeocodeEnabled: Boolean = true,
    @SerializedName("strip_on_export") val stripOnExport: Boolean = false,
)

data class UpdateGeoSettingsRequest(
    @SerializedName("geo_enabled") val geoEnabled: Boolean? = null,
    @SerializedName("reverse_geocode_enabled") val reverseGeocodeEnabled: Boolean? = null,
    @SerializedName("strip_on_export") val stripOnExport: Boolean? = null,
)

data class GeoScrubResponse(
    val scrubbed: Int,
    val message: String? = null,
)

// ── Locations ───────────────────────────────────────────────────────────────

data class GeoCountry(
    val country: String,
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("city_count") val cityCount: Int = 0,
)

data class GeoCountryListResponse(
    val countries: List<GeoCountry>,
)

data class GeoLocation(
    val country: String,
    val city: String,
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("preview_photo_id") val previewPhotoId: String? = null,
    val latitude: Double? = null,
    val longitude: Double? = null,
)

data class GeoLocationListResponse(
    val locations: List<GeoLocation>,
)

data class GeoLocationPhotosResponse(
    val photos: List<PhotoRecord>,
)

// ── Map ─────────────────────────────────────────────────────────────────────

data class GeoMapPhoto(
    @SerializedName("photo_id") val photoId: String,
    val latitude: Double,
    val longitude: Double,
    @SerializedName("thumb_path") val thumbPath: String? = null,
    @SerializedName("blob_id") val blobId: String? = null,
)

data class GeoMapResponse(
    val photos: List<GeoMapPhoto>,
)

// ── Timeline ────────────────────────────────────────────────────────────────

data class GeoTimelineEntry(
    val year: Int,
    val month: Int? = null,
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("preview_photo_id") val previewPhotoId: String? = null,
)

data class GeoTimelineResponse(
    val entries: List<GeoTimelineEntry>,
)

data class GeoTimelinePhotosResponse(
    val photos: List<PhotoRecord>,
)

// ── Memories ────────────────────────────────────────────────────────────────

data class GeoMemory(
    val id: String,
    val title: String,
    @SerializedName("subtitle") val subtitle: String? = null,
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("preview_photo_id") val previewPhotoId: String? = null,
    @SerializedName("anchor_date") val anchorDate: String? = null,
)

data class GeoMemoryListResponse(
    val memories: List<GeoMemory>,
)

data class GeoMemoryPhotosResponse(
    val photos: List<PhotoRecord>,
)

// ── Trips ───────────────────────────────────────────────────────────────────

data class GeoTrip(
    val id: String,
    val title: String,
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("started_at") val startedAt: String? = null,
    @SerializedName("ended_at") val endedAt: String? = null,
    @SerializedName("preview_photo_id") val previewPhotoId: String? = null,
    @SerializedName("country") val country: String? = null,
    @SerializedName("city") val city: String? = null,
)

data class GeoTripListResponse(
    val trips: List<GeoTrip>,
)

data class GeoTripPhotosResponse(
    val photos: List<PhotoRecord>,
)
