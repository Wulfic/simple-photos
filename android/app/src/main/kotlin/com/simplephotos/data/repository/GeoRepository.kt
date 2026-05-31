/**
 * Geo repository — locations, countries, map, timeline, memories,
 * trips, and geo settings.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class GeoRepository @Inject constructor(private val api: ApiService) {

    suspend fun getSettings(): GeoSettings = api.getGeoSettings()

    suspend fun updateSettings(
        enabled: Boolean? = null,
        scrubOnUpload: Boolean? = null,
    ) {
        api.updateGeoSettings(UpdateGeoSettingsRequest(enabled, scrubOnUpload))
    }

    suspend fun scrub(): GeoScrubResponse = api.scrubGeoData(GeoScrubRequest(confirm = true))

    suspend fun listCountries(): List<GeoCountry> = api.listGeoCountries()

    suspend fun listLocations(): List<GeoLocation> = api.listGeoLocations()

    suspend fun listLocationPhotos(country: String, city: String): List<GeoPhotoSummary> =
        api.listGeoLocationPhotos(country, city)

    suspend fun listMapPhotos(): List<GeoMapPhoto> = api.listGeoMapPhotos()

    suspend fun listTimeline(): List<GeoTimelineEntry> = api.listGeoTimeline()

    suspend fun listTimelineYear(year: Int): List<GeoTimelineEntry> =
        api.listGeoTimelineYear(year)

    suspend fun listTimelineMonthPhotos(year: Int, month: Int): List<GeoPhotoSummary> =
        api.listGeoTimelineMonthPhotos(year, month)

    suspend fun listMemories(): List<GeoMemory> = api.listGeoMemories()

    suspend fun listMemoryPhotos(memoryId: String): List<GeoPhotoSummary> =
        api.listGeoMemoryPhotos(memoryId)

    suspend fun listTrips(): List<GeoTrip> = api.listGeoTrips()

    suspend fun listTripPhotos(tripId: String): List<GeoPhotoSummary> =
        api.listGeoTripPhotos(tripId)
}
