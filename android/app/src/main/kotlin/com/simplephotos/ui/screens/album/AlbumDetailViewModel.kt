package com.simplephotos.ui.screens.album

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.simplephotos.ui.components.SelectionState
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.excludeSecure
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SecureGalleryRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

/**
 * ViewModel for the album detail screen, supporting both user-created albums
 * and virtual "smart" albums (Favorites, Photos, GIFs, Videos).
 */
@HiltViewModel
class AlbumDetailViewModel @Inject constructor(
    savedStateHandle: SavedStateHandle,
    private val albumRepository: AlbumRepository,
    private val photoRepository: PhotoRepository,
    private val secureGalleryRepository: SecureGalleryRepository
) : ViewModel() {

    /** Blob IDs currently inside a secure gallery — hidden from this album's
     *  grid + count so securing a photo removes it here too (#16). */
    private var secureBlobIds: Set<String> = emptySet()

    val albumId: String = savedStateHandle["albumId"] ?: ""

    /** Whether this is a virtual smart album (favorites, photos, gifs, videos) */
    val isSmartAlbum: Boolean = albumId.startsWith("smart-")

    /** Human-readable label for smart albums */
    val smartAlbumLabel: String = when (albumId) {
        "smart-recents" -> "Recently Added"
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
    private val selection = SelectionState()
    val selectedIds get() = selection.selectedIds
    val isSelectionMode get() = selection.isSelectionMode

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
                refreshSecureBlobIds()
                // Shared resolver — same source the viewer pager uses. Secure-
                // excluded so a secured favorite/photo/video doesn't reappear
                // inside its smart album after being hidden from the gallery (#16).
                photos = photoRepository.getAlbumPhotos(albumId).excludeSecure(secureBlobIds)
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
                refreshSecureBlobIds()
                album = albumRepository.getAlbum(albumId)
                // Shared resolver — same source the viewer pager uses, so the
                // tapped tile and the viewer's initial page always agree.
                // Secure-excluded so the grid + count match the main gallery (#16):
                // securing a photo removes it from its albums too.
                photos = photoRepository.getAlbumPhotos(albumId).excludeSecure(secureBlobIds)
            } catch (e: Exception) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }

    /** Refresh the set of blob IDs inside secure galleries (best-effort). */
    private suspend fun refreshSecureBlobIds() {
        try {
            secureBlobIds = withContext(Dispatchers.IO) { secureGalleryRepository.getSecureBlobIds() }
        } catch (_: Exception) { /* endpoint unavailable — keep existing set */ }
    }

    fun openAddPanel() {
        viewModelScope.launch {
            val existingIds = photos.map { it.localId }.toSet()
            // Don't offer secured photos in the picker — they're hidden from the
            // regular library, so adding them to an album would be surprising (#16).
            allPhotos = photoRepository.getAllPhotos().first()
                .excludeSecure(secureBlobIds)
                .filter { it.localId !in existingIds }
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

    fun enterSelectionMode(id: String) = selection.enter(id)

    fun toggleSelect(id: String) = selection.toggle(id)

    fun selectAll() = selection.setSelection(photos.map { it.localId }.toSet())

    fun clearSelection() = selection.clear()

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
