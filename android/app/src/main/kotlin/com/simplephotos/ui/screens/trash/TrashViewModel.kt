package com.simplephotos.ui.screens.trash

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.TrashItemDto
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

/** Manages the trash bin: fetching trashed items, restoring, permanent deletion, and empty-all. */
@HiltViewModel
class TrashViewModel @Inject constructor(
    private val api: ApiService,
    private val photoRepository: PhotoRepository,
    private val authRepository: AuthRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    var items by mutableStateOf<List<TrashItemDto>>(emptyList())
        private set
    var isLoading by mutableStateOf(true)
        private set
    var error by mutableStateOf<String?>(null)
    var actionLoading by mutableStateOf<String?>(null)
        private set
    var selectedIds by mutableStateOf(emptySet<String>())
        private set
    var isSelectionMode by mutableStateOf(false)
        private set
    var serverBaseUrl by mutableStateOf("")
        private set
    var username by mutableStateOf("")
        private set

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val prefs = dataStore.data.first()
                username = prefs[KEY_USERNAME] ?: ""
            } catch (_: Exception) {}
            loadTrash()
        }
    }

    fun loadTrash() {
        viewModelScope.launch {
            isLoading = true
            error = null
            try {
                val resp = withContext(Dispatchers.IO) { api.listTrash(limit = 500) }
                items = resp.items
            } catch (e: Exception) {
                error = e.message ?: "Failed to load trash"
            } finally {
                isLoading = false
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

    fun clearSelection() {
        selectedIds = emptySet()
        isSelectionMode = false
    }

    fun emptyTrash() {
        viewModelScope.launch {
            actionLoading = "empty"
            try {
                withContext(Dispatchers.IO) { api.emptyTrash() }
                items = emptyList()
                clearSelection()
            } catch (e: Exception) {
                error = e.message ?: "Failed to empty trash"
            } finally {
                actionLoading = null
            }
        }
    }

    fun restoreSelected() {
        viewModelScope.launch {
            actionLoading = "bulk-restore"
            try {
                withContext(Dispatchers.IO) {
                    coroutineScope {
                        selectedIds.map { id -> async { api.restoreFromTrash(id) } }.awaitAll()
                    }
                }
                items = items.filter { it.id !in selectedIds }
                clearSelection()
            } catch (e: Exception) {
                error = e.message ?: "Failed to restore"
                loadTrash()
            } finally {
                actionLoading = null
            }
        }
    }

    fun deleteSelected() {
        viewModelScope.launch {
            actionLoading = "bulk-delete"
            try {
                withContext(Dispatchers.IO) {
                    coroutineScope {
                        selectedIds.map { id -> async { api.permanentDeleteTrash(id) } }.awaitAll()
                    }
                }
                items = items.filter { it.id !in selectedIds }
                clearSelection()
            } catch (e: Exception) {
                error = e.message ?: "Failed to delete"
                loadTrash()
            } finally {
                actionLoading = null
            }
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
}
