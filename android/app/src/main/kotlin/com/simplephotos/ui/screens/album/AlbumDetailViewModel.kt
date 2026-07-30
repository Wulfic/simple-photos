package com.simplephotos.ui.screens.album

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.simplephotos.ui.components.SelectionState
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.album.AlbumPhotoResolver
import com.simplephotos.data.album.AlbumSort
import com.simplephotos.data.album.AlbumSortField
import com.simplephotos.data.album.DEFAULT_ALBUM_SORT
import com.simplephotos.data.album.nextSort
import com.simplephotos.data.album.sortAlbumPhotos
import com.simplephotos.data.excludeSecure
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
    private val photoRepository: PhotoRepository,
    private val resolver: AlbumPhotoResolver,
) : ViewModel() {

    /** Blob IDs currently inside a secure gallery — hidden from this album's
     *  grid + count so securing a photo removes it here too (#16). Reported by
     *  the resolver so the picker hides exactly the set the grid did. */
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

    // ── Sort (#52) ────────────────────────────────────────────────────────
    // The intrinsic-order (secure-excluded) list as loaded; the displayed
    // `photos` is this re-ordered by the user's choice. Held separately so a
    // sort change re-orders without a reload.
    private var basePhotos: List<PhotoEntity> = emptyList()

    /** User's chosen sort, or null (intrinsic order — e.g. Recently Added's
     *  add-order). null keeps exactly the pre-#52 ordering. */
    var sort by mutableStateOf<AlbumSort?>(null)
        private set

    /** Concrete sort for the header control's visual state. */
    val displaySort: AlbumSort get() = sort ?: DEFAULT_ALBUM_SORT

    var error by mutableStateOf<String?>(null)
    var showAddPanel by mutableStateOf(false)
    var selectedToAdd by mutableStateOf<Set<String>>(emptySet())
    var showDeleteConfirm by mutableStateOf(false)
    var showRenameDialog by mutableStateOf(false)

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
        viewModelScope.launch { loadAlbum() }
    }

    /** Re-derive the displayed list from the intrinsic-order base + current sort.
     *  The comparator itself lives in [sortAlbumPhotos] so the viewer's pager
     *  applies the identical one to the identical base (E3). */
    private fun applySort() {
        photos = sortAlbumPhotos(basePhotos, sort)
    }

    /** Header control tapped a field: toggle direction if active, else switch to
     *  it. Persists the choice and re-orders the grid without a reload (#52). */
    fun selectSortField(field: AlbumSortField) {
        val next = nextSort(displaySort, field)
        sort = next
        applySort()
        viewModelScope.launch {
            try {
                resolver.persistSort(albumId, next)
            } catch (e: Exception) {
                // The sort still applies this session; only persistence is lost.
                // The viewer reads the persisted value, so a failure here also
                // means the pager keeps the PREVIOUS order until the next reload.
                android.util.Log.w("AlbumDetailViewModel", "could not persist sort", e)
            }
        }
    }

    /**
     * Load the album through [AlbumPhotoResolver] — the same call the viewer's
     * pager makes, so the grid and the pager are the same list rather than two
     * derivations of one query (E3). Smart and regular albums take the same
     * path; the resolver handles the kind, the secure exclusion (#16), the burst
     * policy and the persisted #52 sort.
     */
    fun loadAlbum() {
        viewModelScope.launch {
            loading = true
            try {
                if (!isSmartAlbum) album = albumRepository.getAlbum(albumId)
                val resolved = resolver.resolve(albumId)
                basePhotos = resolved.tiles
                secureBlobIds = resolved.secureBlobIds
                sort = resolved.sort
                photos = resolved.photos
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
                albumRepository.addPhotosToAlbum(selectedToAdd.toList(), albumId)
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

    /** Rename this album (#35). No-op for a blank/unchanged name. Optimistically
     *  updates the title on success; logs + surfaces the error on failure. */
    fun renameAlbum(newName: String) {
        val a = album ?: run { showRenameDialog = false; return }
        val trimmed = newName.trim()
        if (trimmed.isEmpty() || trimmed == a.name) { showRenameDialog = false; return }
        viewModelScope.launch {
            try {
                album = albumRepository.renameAlbum(a, trimmed)
            } catch (e: Exception) {
                error = e.message
                android.util.Log.e("AlbumDetailViewModel", "rename album failed", e)
            }
            showRenameDialog = false
        }
    }

    fun enterSelectionMode(id: String) = selection.enter(id)

    fun toggleSelect(id: String) = selection.toggle(id)

    fun selectAll() = selection.setSelection(photos.map { it.localId }.toSet())

    fun clearSelection() = selection.clear()

    fun removeSelectedFromAlbum() {
        viewModelScope.launch {
            try {
                albumRepository.removePhotosFromAlbum(selectedIds.toList(), albumId)
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
