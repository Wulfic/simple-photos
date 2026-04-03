package com.simplephotos.ui.screens.album

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SharedAlbumInfo
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.AuthRepository
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
            // Sync album manifests from server (picks up web-created albums)
            try {
                withContext(Dispatchers.IO) { albumRepository.syncAlbumsFromServer() }
            } catch (_: Exception) {}
            // Load photo counts for smart albums
            loadSmartAlbumCounts()
            // Load shared albums (displayed at the bottom of the albums page)
            loadSharedAlbums()
        }
    }

    private fun loadSmartAlbumCounts() {
        viewModelScope.launch {
            try {
                val allPhotos = withContext(Dispatchers.IO) {
                    photoRepository.getAllPhotos().first()
                }
                totalCount = allPhotos.size
                favoritesCount = allPhotos.count { it.isFavorite }
                photosCount = allPhotos.count { it.mediaType == "photo" || it.mediaType == "gif" }
                gifsCount = allPhotos.count { it.mediaType == "gif" }
                videosCount = allPhotos.count { it.mediaType == "video" }
                audioCount = allPhotos.count { it.mediaType == "audio" }

                // Load cover photos for smart albums (most recent photo matching each filter)
                val sorted = allPhotos.sortedByDescending { it.takenAt }
                val covers = mutableMapOf<String, PhotoEntity>()
                sorted.firstOrNull { it.isFavorite }?.let { covers["smart-favorites"] = it }
                sorted.firstOrNull { it.mediaType == "photo" || it.mediaType == "gif" }?.let { covers["smart-photos"] = it }
                sorted.firstOrNull { it.mediaType == "gif" }?.let { covers["smart-gifs"] = it }
                sorted.firstOrNull { it.mediaType == "video" }?.let { covers["smart-videos"] = it }
                sorted.firstOrNull { it.mediaType == "audio" }?.let { covers["smart-audio"] = it }
                smartAlbumCoverPhotos = covers
            } catch (_: Exception) {}
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
