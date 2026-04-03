package com.simplephotos.ui.screens.sharing

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.remote.dto.SharedAlbumInfo
import com.simplephotos.data.remote.dto.SharedAlbumMember
import com.simplephotos.data.remote.dto.SharedAlbumPhoto
import com.simplephotos.data.remote.dto.ShareableUser
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.SharingRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

/**
 * ViewModel for shared album management: list, create, delete albums;
 * manage members; view photos in each album.
 */
@HiltViewModel
class SharedAlbumsViewModel @Inject constructor(
    private val sharingRepository: SharingRepository,
    private val authRepository: AuthRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {

    // ── Album list ───────────────────────────────────────────────────────
    var albums by mutableStateOf<List<SharedAlbumInfo>>(emptyList())
        private set
    var loading by mutableStateOf(true)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var username by mutableStateOf("")
        private set

    // ── Create dialog ────────────────────────────────────────────────────
    var showCreateDialog by mutableStateOf(false)
    var newAlbumName by mutableStateOf("")

    // ── Detail view ──────────────────────────────────────────────────────
    var selectedAlbum by mutableStateOf<SharedAlbumInfo?>(null)
        private set
    var members by mutableStateOf<List<SharedAlbumMember>>(emptyList())
        private set
    var photos by mutableStateOf<List<SharedAlbumPhoto>>(emptyList())
        private set
    var detailLoading by mutableStateOf(false)
        private set

    // ── Add member picker ────────────────────────────────────────────────
    var showAddMemberDialog by mutableStateOf(false)
    var availableUsers by mutableStateOf<List<ShareableUser>>(emptyList())
        private set

    // ── Delete confirmation ──────────────────────────────────────────────
    var albumToDelete by mutableStateOf<SharedAlbumInfo?>(null)

    init {
        loadAlbums()
        viewModelScope.launch {
            username = dataStore.data.first()[KEY_USERNAME] ?: ""
        }
    }

    fun loadAlbums() {
        viewModelScope.launch {
            loading = true
            error = null
            try {
                albums = withContext(Dispatchers.IO) { sharingRepository.listAlbums() }
            } catch (e: Exception) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }

    fun createAlbum() {
        val name = newAlbumName.trim()
        if (name.isBlank()) return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { sharingRepository.createAlbum(name) }
                newAlbumName = ""
                showCreateDialog = false
                loadAlbums()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun confirmDeleteAlbum() {
        val album = albumToDelete ?: return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { sharingRepository.deleteAlbum(album.id) }
                albumToDelete = null
                if (selectedAlbum?.id == album.id) selectedAlbum = null
                loadAlbums()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    // ── Detail ───────────────────────────────────────────────────────────

    fun selectAlbum(album: SharedAlbumInfo) {
        selectedAlbum = album
        loadDetail(album.id)
    }

    fun closeDetail() {
        selectedAlbum = null
        members = emptyList()
        photos = emptyList()
    }

    private fun loadDetail(albumId: String) {
        viewModelScope.launch {
            detailLoading = true
            try {
                val (m, p) = withContext(Dispatchers.IO) {
                    coroutineScope {
                        val deferredMembers = async { sharingRepository.listMembers(albumId) }
                        val deferredPhotos = async { sharingRepository.listPhotos(albumId) }
                        Pair(deferredMembers.await(), deferredPhotos.await())
                    }
                }
                members = m
                photos = p
            } catch (e: Exception) {
                error = e.message
            } finally {
                detailLoading = false
            }
        }
    }

    // ── Members ──────────────────────────────────────────────────────────

    fun openAddMemberDialog() {
        viewModelScope.launch {
            try {
                availableUsers = withContext(Dispatchers.IO) { sharingRepository.listUsersForSharing() }
                showAddMemberDialog = true
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun addMember(userId: String) {
        val albumId = selectedAlbum?.id ?: return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { sharingRepository.addMember(albumId, userId) }
                showAddMemberDialog = false
                loadDetail(albumId)
                loadAlbums() // refresh member count
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun removeMember(userId: String) {
        val albumId = selectedAlbum?.id ?: return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { sharingRepository.removeMember(albumId, userId) }
                loadDetail(albumId)
                loadAlbums()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    // ── Photos ───────────────────────────────────────────────────────────

    fun removePhoto(photoId: String) {
        val albumId = selectedAlbum?.id ?: return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { sharingRepository.removePhoto(albumId, photoId) }
                loadDetail(albumId)
                loadAlbums()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun logout(onLogout: () -> Unit) {
        viewModelScope.launch {
            try { withContext(Dispatchers.IO) { authRepository.logout() } } catch (_: Exception) {}
            onLogout()
        }
    }
}
