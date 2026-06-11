/**
 * Geo feature DTOs — locations, countries, map, timeline, memories, trips,
 * and geo settings (incl. scrub).
 */
package com.simplephotos.data.remote.dto

import com.google.gson.annotations.SerializedName

// ── Settings ────────────────────────────────────────────────────────────────

// Mirrors server `GeoStatusResponse` (GET /api/settings/geo).
data class GeoSettings(
    val enabled: Boolean = false,
    @SerializedName("scrub_on_upload") val scrubOnUpload: Boolean = false,
    @SerializedName("photos_with_location") val photosWithLocation: Int = 0,
    @SerializedName("photos_without_location") val photosWithoutLocation: Int = 0,
    @SerializedName("unique_countries") val uniqueCountries: Int = 0,
    @SerializedName("unique_cities") val uniqueCities: Int = 0,
)

// Server `GeoSettingsRequest` (POST /api/settings/geo, returns empty 200).
data class UpdateGeoSettingsRequest(
    val enabled: Boolean? = null,
    @SerializedName("scrub_on_upload") val scrubOnUpload: Boolean? = null,
)

// Server `ScrubConfirmRequest` (POST /api/geo/scrub) — must confirm.
data class GeoScrubRequest(
    val confirm: Boolean = true,
)

// Server scrub returns `{ scrubbed_photos }`.
data class GeoScrubResponse(
    @SerializedName("scrubbed_photos") val scrubbedPhotos: Int = 0,
)

// Bare PhotoSummary array returned by geo photo-list endpoints.
data class GeoPhotoSummary(
    val id: String,
    val filename: String? = null,
    @SerializedName("thumb_path") val thumbPath: String? = null,
    @SerializedName("taken_at") val takenAt: String? = null,
    val latitude: Double? = null,
    val longitude: Double? = null,
)

// ── Locations ───────────────────────────────────────────────────────────────

// Mirrors server `CountryEntry` (bare array from GET /api/geo/countries).
data class GeoCountry(
    val country: String,
    @SerializedName("country_code") val countryCode: String? = null,
    @SerializedName("photo_count") val photoCount: Int,
)

// Mirrors server `LocationEntry` (bare array from GET /api/geo/locations).
data class GeoLocation(
    val city: String,
    val state: String? = null,
    val country: String,
    @SerializedName("country_code") val countryCode: String? = null,
    @SerializedName("photo_count") val photoCount: Int,
)

// ── Map ─────────────────────────────────────────────────────────────────────

// Mirrors server `PhotoSummary` (bare array from GET /api/geo/map).
data class GeoMapPhoto(
    @SerializedName("id") val photoId: String,
    val filename: String? = null,
    @SerializedName("thumb_path") val thumbPath: String? = null,
    @SerializedName("taken_at") val takenAt: String? = null,
    val latitude: Double = 0.0,
    val longitude: Double = 0.0,
)

// ── Timeline ────────────────────────────────────────────────────────────────

// Mirrors server `TimelineYearEntry` / `TimelineMonthEntry` (bare arrays).
data class GeoTimelineEntry(
    val year: Int,
    val month: Int? = null,
    @SerializedName("photo_count") val photoCount: Int,
)

// ── Memories ────────────────────────────────────────────────────────────────

// Mirrors server `Memory` (bare array from GET /api/geo/memories).
data class GeoMemory(
    val id: String,
    val name: String,
    val city: String = "",
    val country: String = "",
    @SerializedName("date_label") val dateLabel: String = "",
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("first_photo_id") val firstPhotoId: String? = null,
    @SerializedName("first_thumb_path") val firstThumbPath: String? = null,
) {
    /** Location line, derived from city/country (server sends no subtitle). */
    val subtitle: String?
        get() = listOfNotNull(city, country).filter { it.isNotEmpty() }
            .joinToString(", ").ifEmpty { null }
}

// ── Trips ───────────────────────────────────────────────────────────────────

// Mirrors server `Trip` (bare array from GET /api/geo/trips).
data class GeoTrip(
    val id: String,
    val name: String,
    val city: String = "",
    val state: String? = null,
    val country: String = "",
    @SerializedName("country_code") val countryCode: String? = null,
    @SerializedName("start_date") val startDate: String? = null,
    @SerializedName("end_date") val endDate: String? = null,
    @SerializedName("date_label") val dateLabel: String = "",
    @SerializedName("photo_count") val photoCount: Int,
    @SerializedName("day_count") val dayCount: Int = 0,
    @SerializedName("first_photo_id") val firstPhotoId: String? = null,
    @SerializedName("first_thumb_path") val firstThumbPath: String? = null,
)
