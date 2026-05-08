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
        geoEnabled: Boolean? = null,
        reverseGeocodeEnabled: Boolean? = null,
        stripOnExport: Boolean? = null,
    ): GeoSettings = api.updateGeoSettings(
        UpdateGeoSettingsRequest(geoEnabled, reverseGeocodeEnabled, stripOnExport)
    )

    suspend fun scrub(): GeoScrubResponse = api.scrubGeoData()

    suspend fun listCountries(): List<GeoCountry> = api.listGeoCountries().countries

    suspend fun listLocations(): List<GeoLocation> = api.listGeoLocations().locations

    suspend fun listLocationPhotos(country: String, city: String): List<PhotoRecord> =
        api.listGeoLocationPhotos(country, city).photos

    suspend fun listMapPhotos(): List<GeoMapPhoto> = api.listGeoMapPhotos().photos

    suspend fun listTimeline(): List<GeoTimelineEntry> = api.listGeoTimeline().entries

    suspend fun listTimelineYear(year: Int): List<GeoTimelineEntry> =
        api.listGeoTimelineYear(year).entries

    suspend fun listTimelineMonthPhotos(year: Int, month: Int): List<PhotoRecord> =
        api.listGeoTimelineMonthPhotos(year, month).photos

    suspend fun listMemories(): List<GeoMemory> = api.listGeoMemories().memories

    suspend fun listMemoryPhotos(memoryId: String): List<PhotoRecord> =
        api.listGeoMemoryPhotos(memoryId).photos

    suspend fun listTrips(): List<GeoTrip> = api.listGeoTrips().trips

    suspend fun listTripPhotos(tripId: String): List<PhotoRecord> =
        api.listGeoTripPhotos(tripId).photos
}
