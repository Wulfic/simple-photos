package com.simplephotos.ui.screens.search

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.album.GridPhotoIds
import com.simplephotos.data.album.gridPhotoIds
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.SearchResult
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

/** Drives server-side photo search by filename, applying results to the local gallery. */
@HiltViewModel
class SearchViewModel @Inject constructor(
    private val api: ApiService,
    private val photoRepository: PhotoRepository,
    private val authRepository: AuthRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {

    var query by mutableStateOf("")
        private set
    var results by mutableStateOf<List<SearchResult>>(emptyList())
        private set
    var isLoading by mutableStateOf(false)
        private set
    var searched by mutableStateOf(false)
        private set
    var serverBaseUrl by mutableStateOf("")
        private set
    var username by mutableStateOf("")
        private set
    /** Map of server photo ID → local PhotoEntity for cached thumbnail lookup */
    var localPhotoMap by mutableStateOf<Map<String, PhotoEntity>>(emptyMap())
        private set

    /**
     * The same lookup projected into the two id spaces the viewer handoff needs
     * (#52, E3a) — server ids for the tiles, local ids for the pager, both in
     * relevance order. Built from the same [PhotoEntity] list as [localPhotoMap],
     * not a second query.
     *
     * Search is the surface where the handoff matters most: `/api/search` ranks
     * by relevance, and the gallery branch the viewer used to fall back to is
     * `takenAt DESC`, so swiping off the first hit left the result set entirely.
     */
    var grid by mutableStateOf(GridPhotoIds.EMPTY)
        private set

    private var searchJob: Job? = null

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val prefs = dataStore.data.first()
                username = prefs[KEY_USERNAME] ?: ""
            } catch (_: Exception) {}
        }
    }

    fun updateQuery(newQuery: String) {
        query = newQuery
        searchJob?.cancel()
        if (newQuery.isBlank()) {
            results = emptyList()
            localPhotoMap = emptyMap()
            searched = false
            return
        }
        searchJob = viewModelScope.launch {
            delay(300) // debounce
            doSearch(newQuery)
        }
    }

    private suspend fun doSearch(q: String) {
        isLoading = true
        searched = true
        try {
            val response = withContext(Dispatchers.IO) { api.searchPhotos(q.trim()) }
            results = response.results

            // Batch-load local photos for thumbnail resolution
            if (response.results.isNotEmpty()) {
                val ids = response.results.map { it.id }
                val localPhotos = withContext(Dispatchers.IO) {
                    photoRepository.getPhotosByServerPhotoIds(ids)
                }
                localPhotoMap = localPhotos.associateBy { it.serverPhotoId ?: "" }
                grid = gridPhotoIds(ids, localPhotos)
            } else {
                localPhotoMap = emptyMap()
                grid = GridPhotoIds.EMPTY
            }
        } catch (_: Exception) {
            results = emptyList()
            localPhotoMap = emptyMap()
            grid = GridPhotoIds.EMPTY
        } finally {
            isLoading = false
        }
    }

    fun logout(onLogout: () -> Unit) {
        viewModelScope.launch {
            try { authRepository.logout() } catch (_: Exception) {}
            onLogout()
        }
    }
}
