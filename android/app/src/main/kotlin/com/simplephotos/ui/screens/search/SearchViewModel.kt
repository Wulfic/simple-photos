package com.simplephotos.ui.screens.search

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
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
    var allTags by mutableStateOf<List<String>>(emptyList())
        private set
    var isLoading by mutableStateOf(false)
        private set
    var searched by mutableStateOf(false)
        private set
    var serverBaseUrl by mutableStateOf("")
        private set
    var username by mutableStateOf("")
        private set

    private var searchJob: Job? = null

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val prefs = dataStore.data.first()
                username = prefs[KEY_USERNAME] ?: ""
                val tagsResponse = withContext(Dispatchers.IO) { api.listTags() }
                allTags = tagsResponse.tags
            } catch (_: Exception) {}
        }
    }

    fun updateQuery(newQuery: String) {
        query = newQuery
        searchJob?.cancel()
        if (newQuery.isBlank()) {
            results = emptyList()
            searched = false
            return
        }
        searchJob = viewModelScope.launch {
            delay(300) // debounce
            doSearch(newQuery)
        }
    }

    fun searchTag(tag: String) {
        query = tag
        searchJob?.cancel()
        searchJob = viewModelScope.launch { doSearch(tag) }
    }

    private suspend fun doSearch(q: String) {
        isLoading = true
        searched = true
        try {
            val response = withContext(Dispatchers.IO) { api.searchPhotos(q.trim()) }
            results = response.results
        } catch (_: Exception) {
            results = emptyList()
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
