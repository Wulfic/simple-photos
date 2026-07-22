package com.simplephotos.ui.screens.album

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import com.simplephotos.ui.components.SelectionState
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.album.AlbumSort
import com.simplephotos.data.album.AlbumSortField
import com.simplephotos.data.album.DEFAULT_ALBUM_SORT
import com.simplephotos.data.album.nextSort
import com.simplephotos.data.album.parseAlbumSort
import com.simplephotos.data.album.serialize
import com.simplephotos.data.album.sortAlbumItems
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
    private val secureGalleryRepository: SecureGalleryRepository,
    private val dataStore: DataStore<Preferences>
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

    private val sortPrefKey = stringPreferencesKey("albumSort:$albumId")
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
        viewModelScope.launch {
            // Read the persisted choice before the first render's sort applies.
            // A missing/corrupt value parses to null (intrinsic order).
            sort = try {
                parseAlbumSort(dataStore.data.first()[sortPrefKey])
            } catch (_: Exception) {
                null
            }
            if (isSmartAlbum) loadSmartAlbum() else loadAlbum()
        }
    }

    /** Re-derive the displayed list from the intrinsic-order base + current sort. */
    private fun applySort() {
        val s = sort
        photos = if (s == null) {
            basePhotos
        } else {
            sortAlbumItems(basePhotos, s, { it.takenAt }, { it.filename }, { it.localId })
        }
    }

    /** Header control tapped a field: toggle direction if active, else switch to
     *  it. Persists the choice and re-orders the grid without a reload (#52). */
    fun selectSortField(field: AlbumSortField) {
        val next = nextSort(displaySort, field)
        sort = next
        applySort()
        viewModelScope.launch {
            try {
                dataStore.edit { it[sortPrefKey] = next.serialize() }
            } catch (e: Exception) {
                // The sort still applies this session; only persistence is lost.
                android.util.Log.w("AlbumDetailViewModel", "could not persist sort", e)
            }
        }
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
                basePhotos = photoRepository.getAlbumPhotos(albumId).excludeSecure(secureBlobIds)
                applySort()
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
                basePhotos = photoRepository.getAlbumPhotos(albumId).excludeSecure(secureBlobIds)
                applySort()
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
