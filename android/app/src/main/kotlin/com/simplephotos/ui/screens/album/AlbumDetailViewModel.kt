package com.simplephotos.ui.screens.album

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.PhotoRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import javax.inject.Inject

/**
 * ViewModel for the album detail screen, supporting both user-created albums
 * and virtual "smart" albums (Favorites, Photos, GIFs, Videos).
 */
@HiltViewModel
class AlbumDetailViewModel @Inject constructor(
    savedStateHandle: SavedStateHandle,
    private val albumRepository: AlbumRepository,
    private val photoRepository: PhotoRepository
) : ViewModel() {

    val albumId: String = savedStateHandle["albumId"] ?: ""

    /** Whether this is a virtual smart album (favorites, photos, gifs, videos) */
    val isSmartAlbum: Boolean = albumId.startsWith("smart-")

    /** Human-readable label for smart albums */
    val smartAlbumLabel: String = when (albumId) {
        "smart-favorites" -> "Favorites"
        "smart-photos" -> "Photos"
        "smart-gifs" -> "GIFs"
        "smart-videos" -> "Videos"
        "smart-audio" -> "Audio"
        else -> "Album"
    }

    var album by mutableStateOf<AlbumEntity?>(null)
    var photos by mutableStateOf<List<PhotoEntity>>(emptyList())
    var allPhotos by mutableStateOf<List<PhotoEntity>>(emptyList())
    var loading by mutableStateOf(true)
    var error by mutableStateOf<String?>(null)
    var showAddPanel by mutableStateOf(false)
    var selectedToAdd by mutableStateOf<Set<String>>(emptySet())
    var showDeleteConfirm by mutableStateOf(false)

    var serverBaseUrl by mutableStateOf("")
        private set


    // ── Multi-select state ────────────────────────────────────────
    var selectedIds by mutableStateOf(emptySet<String>())
        private set
    var isSelectionMode by mutableStateOf(false)
        private set

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = photoRepository.getServerBaseUrl()
            } catch (_: Exception) {}
        }
        if (isSmartAlbum) loadSmartAlbum() else loadAlbum()
    }

    /** Load filtered photos for smart albums */
    private fun loadSmartAlbum() {
        viewModelScope.launch {
            loading = true
            try {
                val all = photoRepository.getAllPhotos().first()
                photos = when (albumId) {
                    "smart-favorites" -> all.filter { it.isFavorite }
                    "smart-photos" -> all.filter { it.mediaType == "photo" || it.mediaType == "gif" }
                    "smart-gifs" -> all.filter { it.mediaType == "gif" }
                    "smart-videos" -> all.filter { it.mediaType == "video" }
                    "smart-audio" -> all.filter { it.mediaType == "audio" }
                    else -> all
                }
            } catch (e: Exception) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }

    fun loadAlbum() {
        viewModelScope.launch {
            loading = true
            try {
                album = albumRepository.getAlbum(albumId)
                val photoIds = albumRepository.getPhotoIdsForAlbum(albumId)
                photos = photoIds.mapNotNull { id ->
                    photoRepository.getPhoto(id)
                }
            } catch (e: Exception) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }

    fun openAddPanel() {
        viewModelScope.launch {
            val existingIds = photos.map { it.localId }.toSet()
            allPhotos = photoRepository.getAllPhotos().first().filter { it.localId !in existingIds }
            selectedToAdd = emptySet()
            showAddPanel = true
        }
    }

    fun toggleSelection(photoId: String) {
        selectedToAdd = if (photoId in selectedToAdd) {
            selectedToAdd - photoId
        } else {
            selectedToAdd + photoId
        }
    }

    fun selectAllAvailable() {
        selectedToAdd = allPhotos.map { it.localId }.toSet()
    }

    fun confirmAdd() {
        viewModelScope.launch {
            try {
                selectedToAdd.forEach { photoId ->
                    albumRepository.addPhotoToAlbum(photoId, albumId)
                }
                // Only sync album manifest in encrypted mode
                try {
                    album?.let { albumRepository.syncAlbum(it) }
                } catch (_: Exception) {
                    // Sync may fail — album data is still stored locally
                }
                showAddPanel = false
                loadAlbum()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun removePhoto(photoId: String) {
        viewModelScope.launch {
            try {
                albumRepository.removePhotoFromAlbum(photoId, albumId)
                try {
                    album?.let { albumRepository.syncAlbum(it) }
                } catch (_: Exception) {}
                loadAlbum()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun deleteAlbum(onDeleted: () -> Unit) {
        viewModelScope.launch {
            try {
                album?.let { albumRepository.deleteAlbum(it) }
                onDeleted()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun enterSelectionMode(id: String) {
        isSelectionMode = true
        selectedIds = setOf(id)
    }

    fun toggleSelect(id: String) {
        if (!isSelectionMode) return
        selectedIds = if (id in selectedIds) selectedIds - id else selectedIds + id
        if (selectedIds.isEmpty()) isSelectionMode = false
    }

    fun selectAll() {
        isSelectionMode = true
        selectedIds = photos.map { it.localId }.toSet()
    }

    fun clearSelection() {
        selectedIds = emptySet()
        isSelectionMode = false
    }

    fun removeSelectedFromAlbum() {
        viewModelScope.launch {
            try {
                for (id in selectedIds) {
                    albumRepository.removePhotoFromAlbum(id, albumId)
                }
                try {
                    album?.let { albumRepository.syncAlbum(it) }
                } catch (_: Exception) {}
                clearSelection()
                loadAlbum()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }
}
