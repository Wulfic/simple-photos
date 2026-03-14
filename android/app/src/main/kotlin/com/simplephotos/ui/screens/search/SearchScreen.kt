package com.simplephotos.ui.screens.search

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.R
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.SearchResult
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.ui.components.ActiveTab
import com.simplephotos.ui.components.AppHeader
import com.simplephotos.ui.components.HeaderNavigation
import com.simplephotos.ui.theme.ThemeState
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

// ── ViewModel ───────────────────────────────────────────────────────────────

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

// ── Screen ──────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun SearchScreen(
    onPhotoClick: (String) -> Unit,
    onGalleryClick: () -> Unit,
    onAlbumsClick: () -> Unit,
    onTrashClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onSecureGalleryClick: () -> Unit = {},
    onSharedAlbumsClick: () -> Unit = {},
    onDiagnosticsClick: () -> Unit = {},
    onLogout: () -> Unit,
    isAdmin: Boolean = false,
    viewModel: SearchViewModel = hiltViewModel()
) {
    val focusRequester = remember { FocusRequester() }
    val isSystemDark = androidx.compose.foundation.isSystemInDarkTheme()

    LaunchedEffect(Unit) {
        focusRequester.requestFocus()
    }

    Scaffold(
        topBar = {
            AppHeader(
                activeTab = ActiveTab.SEARCH,
                username = viewModel.username,
                navigation = HeaderNavigation(
                    onGalleryClick = onGalleryClick,
                    onAlbumsClick = onAlbumsClick,
                    onSearchClick = { /* already on search */ },
                    onTrashClick = onTrashClick,
                    onSettingsClick = onSettingsClick,
                    onSecureGalleryClick = onSecureGalleryClick,
                    onSharedAlbumsClick = onSharedAlbumsClick,
                    onDiagnosticsClick = onDiagnosticsClick,
                    onLogout = { viewModel.logout(onLogout) },
                    onToggleTheme = { ThemeState.toggle(viewModel.dataStore, ThemeState.isDark(isSystemDark)) },
                    isAdmin = isAdmin
                )
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
        ) {
            // Search input
            OutlinedTextField(
                value = viewModel.query,
                onValueChange = { viewModel.updateQuery(it) },
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .focusRequester(focusRequester),
                placeholder = { Text("Search tags, filenames, dates, types…") },
                leadingIcon = {
                    Icon(
                        painter = painterResource(R.drawable.ic_magnify_glass),
                        contentDescription = "Search",
                        modifier = Modifier.size(20.dp)
                    )
                },
                trailingIcon = {
                    if (viewModel.query.isNotEmpty()) {
                        IconButton(onClick = { viewModel.updateQuery("") }) {
                            Text("✕", fontSize = 16.sp)
                        }
                    }
                },
                singleLine = true,
                shape = RoundedCornerShape(16.dp)
            )

            // Tag cloud when no query
            AnimatedVisibility(visible = viewModel.query.isEmpty() && viewModel.allTags.isNotEmpty()) {
                Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
                    Text(
                        "Your tags",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.height(8.dp))
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        viewModel.allTags.forEach { tag ->
                            Surface(
                                modifier = Modifier.clickable { viewModel.searchTag(tag) },
                                shape = CircleShape,
                                color = MaterialTheme.colorScheme.primaryContainer
                            ) {
                                Text(
                                    text = tag,
                                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                                    style = MaterialTheme.typography.bodySmall,
                                    fontWeight = FontWeight.Medium,
                                    color = MaterialTheme.colorScheme.onPrimaryContainer
                                )
                            }
                        }
                    }
                }
            }

            // Loading
            if (viewModel.isLoading) {
                Box(
                    modifier = Modifier.fillMaxWidth().padding(top = 32.dp),
                    contentAlignment = Alignment.Center
                ) {
                    CircularProgressIndicator()
                }
            }

            // No results
            if (viewModel.searched && !viewModel.isLoading && viewModel.results.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxWidth().padding(top = 32.dp),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            "No results found",
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            "Try a different tag or filename",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f)
                        )
                    }
                }
            }

            // Results count
            if (viewModel.results.isNotEmpty()) {
                Text(
                    "${viewModel.results.size} result${if (viewModel.results.size != 1) "s" else ""}",
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            // Results grid
            LazyVerticalGrid(
                columns = GridCells.Adaptive(minSize = 100.dp),
                contentPadding = PaddingValues(4.dp),
                modifier = Modifier.fillMaxSize()
            ) {
                items(viewModel.results) { result ->
                    SearchResultTile(
                        result = result,
                        serverBaseUrl = viewModel.serverBaseUrl,
                        onClick = { onPhotoClick(result.id) }
                    )
                }
            }

            // Empty state when no tags at all
            if (viewModel.query.isEmpty() && viewModel.allTags.isEmpty() && !viewModel.isLoading) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Icon(
                            painter = painterResource(R.drawable.ic_magnify_glass),
                            contentDescription = null,
                            modifier = Modifier.size(48.dp),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.3f)
                        )
                        Spacer(Modifier.height(12.dp))
                        Text(
                            "No tags yet",
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            "Open a photo and add tags to\nstart organizing your library",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                            textAlign = TextAlign.Center
                        )
                    }
                }
            }
        }
    }
}

// ── Result tile ─────────────────────────────────────────────────────────────

@Composable
private fun SearchResultTile(
    result: SearchResult,
    serverBaseUrl: String,
    onClick: () -> Unit
) {
    val context = LocalContext.current
    val thumbUrl = "$serverBaseUrl/api/photos/${result.id}/thumb"

    Box(
        modifier = Modifier
            .padding(2.dp)
            .aspectRatio(1f)
            .clip(RoundedCornerShape(4.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable(onClick = onClick)
    ) {
        AsyncImage(
            model = ImageRequest.Builder(context)
                .data(thumbUrl)
                .crossfade(true)
                .build(),
            contentDescription = result.filename,
            modifier = Modifier.fillMaxSize(),
            contentScale = ContentScale.Crop
        )

        // Video badge
        if (result.mediaType == "video") {
            Surface(
                modifier = Modifier
                    .align(Alignment.BottomEnd)
                    .padding(4.dp),
                color = Color.Black.copy(alpha = 0.6f),
                shape = RoundedCornerShape(4.dp)
            ) {
                Text(
                    "▶",
                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                    color = Color.White,
                    fontSize = 10.sp
                )
            }
        }

        // GIF badge
        if (result.mediaType == "gif") {
            Surface(
                modifier = Modifier
                    .align(Alignment.BottomEnd)
                    .padding(4.dp),
                color = Color.Black.copy(alpha = 0.6f),
                shape = RoundedCornerShape(4.dp)
            ) {
                Text(
                    "GIF",
                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                    color = Color.White,
                    fontSize = 10.sp
                )
            }
        }

        // Tag chips at top
        if (result.tags.isNotEmpty()) {
            Row(
                modifier = Modifier
                    .align(Alignment.TopStart)
                    .padding(4.dp),
                horizontalArrangement = Arrangement.spacedBy(2.dp)
            ) {
                result.tags.take(2).forEach { tag ->
                    Surface(
                        color = Color.Black.copy(alpha = 0.6f),
                        shape = CircleShape
                    ) {
                        Text(
                            text = tag,
                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                            color = Color.White,
                            fontSize = 9.sp,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                }
                if (result.tags.size > 2) {
                    Surface(
                        color = Color.Black.copy(alpha = 0.6f),
                        shape = CircleShape
                    ) {
                        Text(
                            text = "+${result.tags.size - 2}",
                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                            color = Color.White,
                            fontSize = 9.sp
                        )
                    }
                }
            }
        }
    }
}
