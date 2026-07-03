package com.simplephotos.ui.screens.album

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.collapseBursts
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.FaceCluster
import com.simplephotos.data.remote.dto.GeoMemory
import com.simplephotos.data.remote.dto.GeoTrip
import com.simplephotos.data.remote.dto.PetCluster
import com.simplephotos.data.remote.dto.SharedAlbumInfo
import com.simplephotos.data.repository.AiRepository
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.GeoRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SharingRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

/**
 * ViewModel for the album list screen. Loads user albums, smart album counts,
 * shared albums, syncs album manifests from the server, and manages album CRUD.
 */
@HiltViewModel
class AlbumViewModel @Inject constructor(
    private val albumRepository: AlbumRepository,
    private val authRepository: AuthRepository,
    private val photoRepository: PhotoRepository,
    private val sharingRepository: SharingRepository,
    private val aiRepository: AiRepository,
    private val geoRepository: GeoRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    val albums = albumRepository.getAllAlbums()
    var error by mutableStateOf<String?>(null)
    var showCreateDialog by mutableStateOf(false)
    var newAlbumName by mutableStateOf("")
    var username by mutableStateOf("")
        private set

    /** Map of albumId -> first PhotoEntity (for cover image preview) */
    var albumCoverPhotos by mutableStateOf<Map<String, PhotoEntity>>(emptyMap())
        private set

    /** Base URL for server-based thumbnails */
    var serverBaseUrl by mutableStateOf("")
        private set

    /** Photo counts for smart/default albums */
    var totalCount by mutableStateOf(0)
        private set
    /** Count for the "Recently Added" smart album (capped at 100, like web). */
    var recentCount by mutableStateOf(0)
        private set
    var favoritesCount by mutableStateOf(0)
        private set
    var photosCount by mutableStateOf(0)
        private set
    var gifsCount by mutableStateOf(0)
        private set
    var videosCount by mutableStateOf(0)
        private set
    var audioCount by mutableStateOf(0)
        private set

    /** Cover photos for smart albums (keyed by smart album ID) */
    var smartAlbumCoverPhotos by mutableStateOf<Map<String, PhotoEntity>>(emptyMap())
        private set

    // ── Discover sections (people / pets / memories / trips) ────────────
    var peopleClusters by mutableStateOf<List<FaceCluster>>(emptyList())
        private set
    var petClusters by mutableStateOf<List<PetCluster>>(emptyList())
        private set
    var memories by mutableStateOf<List<GeoMemory>>(emptyList())
        private set
    var trips by mutableStateOf<List<GeoTrip>>(emptyList())
        private set

    // ── Shared albums ────────────────────────────────────────────────────
    var sharedAlbums by mutableStateOf<List<SharedAlbumInfo>>(emptyList())
        private set
    var sharedLoading by mutableStateOf(true)
        private set
    var showCreateSharedDialog by mutableStateOf(false)
    var newSharedAlbumName by mutableStateOf("")

    init {
        viewModelScope.launch {
            try {
                val prefs = dataStore.data.first()
                username = prefs[KEY_USERNAME] ?: ""
            } catch (_: Exception) {}
            // Load server config
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
            } catch (_: Exception) {}
            refresh()
        }
    }

    /**
     * Re-sync album manifests from the server and recompute the non-reactive
     * snapshots (smart-album counts/covers, shared albums, Discover sections).
     *
     * The album *list* itself is a reactive Room Flow, but these snapshots were
     * previously computed only once in `init`, so web-created albums and newly
     * synced photos never appeared until the app was restarted (issue #12 —
     * "Albums on Android do not auto-refresh"). The album screen now calls this
     * on every ON_RESUME so re-entering the tab reflects the current state.
     */
    fun refresh() {
        // Sync album manifests from server (picks up web-created albums), then
        // materialize any Google Takeout source albums automatically. Both write
        // to Room, so the reactive `albums` Flow updates the list on its own.
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { albumRepository.syncAlbumsFromServer() }
            } catch (e: Exception) {
                android.util.Log.w("AlbumViewModel", "album sync failed: ${e.message}")
            }
            // Takeout albums are captured server-side at import time and rebuilt
            // into local manifests here automatically — no manual "rebuild" step
            // (Issue 2). Idempotent and best-effort; runs after the server sync so
            // it merges on top rather than fighting it. Photos not synced yet are
            // skipped and picked up on a later refresh.
            try {
                withContext(Dispatchers.IO) { albumRepository.recreateAlbumsFromServer() }
            } catch (e: Exception) {
                android.util.Log.w("AlbumViewModel", "takeout album materialize failed: ${e.message}")
            }
        }
        // Recompute smart-album counts/covers, shared albums, and Discover.
        loadSmartAlbumCounts()
        loadSharedAlbums()
        loadDiscoverSections()
    }

    private fun loadDiscoverSections() {
        viewModelScope.launch {
            try { peopleClusters = withContext(Dispatchers.IO) { aiRepository.listFaceClusters() } } catch (_: Exception) {}
            try { petClusters = withContext(Dispatchers.IO) { aiRepository.listPetClusters() } } catch (_: Exception) {}
            try { memories = withContext(Dispatchers.IO) { geoRepository.listMemories() } } catch (_: Exception) {}
            try { trips = withContext(Dispatchers.IO) { geoRepository.listTrips() } } catch (_: Exception) {}
        }
    }

    private fun loadSmartAlbumCounts() {
        viewModelScope.launch {
            // Fast path (Issue 3): server-precomputed counts render the smart-album
            // badges INSTANTLY on a cold/empty Room mirror — before the local
            // encrypted-sync finishes filling it — instead of showing 0 until a
            // full pagination completes. The burst-collapsed local counts below
            // override these as soon as Room actually holds photos.
            try {
                val summary = withContext(Dispatchers.IO) { photoRepository.apiService.photosSummary() }
                totalCount = summary.total.toInt()
                recentCount = minOf(summary.total, 100L).toInt()
                favoritesCount = summary.favorites.toInt()
                photosCount = (summary.photos + summary.gifs).toInt()
                gifsCount = summary.gifs.toInt()
                videosCount = summary.videos.toInt()
                audioCount = summary.audio.toInt()
            } catch (e: Exception) {
                // Non-fatal: local Room counts below are the fallback. Log so a
                // broken endpoint (stale server) is visible rather than silent.
                android.util.Log.w("AlbumViewModel", "photos/summary fetch failed: ${e.message}")
            }

            try {
                val allPhotos = withContext(Dispatchers.IO) {
                    photoRepository.getAllPhotos().first()
                }
                // Only override the server summary once the local mirror actually
                // holds photos — otherwise an empty cold Room would reset the
                // instant counts above back to 0.
                if (allPhotos.isNotEmpty()) {
                    totalCount = allPhotos.size
                    // "Recently Added" mirrors the web smart album: capped at 100,
                    // with bursts collapsed so a burst counts as one item (the
                    // detail list does the same — see AlbumDetailViewModel).
                    recentCount = minOf(allPhotos.collapseBursts().size, 100)
                    // Counts match the collapsed grids (Favorites/Photos collapse
                    // bursts in getAlbumPhotos) so the card count equals the tiles.
                    favoritesCount = allPhotos.filter { it.isFavorite }.collapseBursts().size
                    photosCount = allPhotos.filter { it.mediaType == "photo" || it.mediaType == "gif" }.collapseBursts().size
                    gifsCount = allPhotos.count { it.mediaType == "gif" }
                    videosCount = allPhotos.count { it.mediaType == "video" }
                    audioCount = allPhotos.count { it.mediaType == "audio" }
                }

                // Load cover photos for smart albums (most recent photo matching each filter)
                val sorted = allPhotos.sortedByDescending { it.takenAt }
                val covers = mutableMapOf<String, PhotoEntity>()
                // "Recently Added" cover = the most recently imported item (by createdAt).
                allPhotos.maxByOrNull { it.createdAt }?.let { covers["smart-recents"] = it }
                sorted.firstOrNull { it.isFavorite }?.let { covers["smart-favorites"] = it }
                sorted.firstOrNull { it.mediaType == "photo" || it.mediaType == "gif" }?.let { covers["smart-photos"] = it }
                sorted.firstOrNull { it.mediaType == "gif" }?.let { covers["smart-gifs"] = it }
                sorted.firstOrNull { it.mediaType == "video" }?.let { covers["smart-videos"] = it }
                sorted.firstOrNull { it.mediaType == "audio" }?.let { covers["smart-audio"] = it }
                smartAlbumCoverPhotos = covers
            } catch (e: Exception) {
                android.util.Log.w("AlbumViewModel", "local smart-album counts failed: ${e.message}")
            }
        }
    }

    /** Load cover photo for each album (call whenever albums list updates). */
    fun loadCoverPhotos(albums: List<AlbumEntity>) {
        viewModelScope.launch {
            val covers = mutableMapOf<String, PhotoEntity>()
            for (album in albums) {
                try {
                    val photoIds = withContext(Dispatchers.IO) { albumRepository.getPhotoIdsForAlbum(album.localId) }
                    val firstId = photoIds.firstOrNull() ?: continue
                    val photo = withContext(Dispatchers.IO) { photoRepository.getPhoto(firstId) }
                    if (photo != null) covers[album.localId] = photo
                } catch (_: Exception) {}
            }
            albumCoverPhotos = covers
        }
    }

    fun logout(onLoggedOut: () -> Unit) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { authRepository.logout() }
            } catch (_: Exception) {}
            onLoggedOut()
        }
    }

    fun createAlbum() {
        val name = newAlbumName.trim()
        if (name.isBlank()) return
        viewModelScope.launch {
            try {
                albumRepository.createAlbum(name)
                newAlbumName = ""
                showCreateDialog = false
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun deleteAlbum(album: AlbumEntity) {
        viewModelScope.launch {
            try {
                albumRepository.deleteAlbum(album)
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    // ── Shared album operations ──────────────────────────────────────────

    /** Fetch shared albums from the server. */
    fun loadSharedAlbums() {
        viewModelScope.launch {
            sharedLoading = true
            try {
                sharedAlbums = withContext(Dispatchers.IO) { sharingRepository.listAlbums() }
            } catch (_: Exception) {
                // Non-fatal: shared albums may not be available
            } finally {
                sharedLoading = false
            }
        }
    }

    /** Create a new shared album and refresh the list. */
    fun createSharedAlbum() {
        val name = newSharedAlbumName.trim()
        if (name.isBlank()) return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { sharingRepository.createAlbum(name) }
                newSharedAlbumName = ""
                showCreateSharedDialog = false
                loadSharedAlbums()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }
}
