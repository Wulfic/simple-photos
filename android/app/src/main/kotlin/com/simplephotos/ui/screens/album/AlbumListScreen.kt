package com.simplephotos.ui.screens.album

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.ui.components.ActiveTab
import com.simplephotos.ui.components.AppHeader
import com.simplephotos.ui.components.HeaderNavigation
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import com.simplephotos.ui.theme.ThemeState
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

@HiltViewModel
class AlbumViewModel @Inject constructor(
    private val albumRepository: AlbumRepository,
    private val authRepository: AuthRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    val albums = albumRepository.getAllAlbums()
    var error by mutableStateOf<String?>(null)
    var showCreateDialog by mutableStateOf(false)
    var newAlbumName by mutableStateOf("")
    var username by mutableStateOf("")
        private set

    init {
        viewModelScope.launch {
            try {
                val prefs = dataStore.data.first()
                username = prefs[KEY_USERNAME] ?: ""
            } catch (_: Exception) {}
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
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AlbumListScreen(
    onGalleryClick: () -> Unit,
    onSearchClick: () -> Unit,
    onTrashClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onLogout: () -> Unit,
    onAlbumClick: (String) -> Unit = {},
    viewModel: AlbumViewModel = hiltViewModel()
) {
    val albums by viewModel.albums.collectAsState(initial = emptyList())

    Scaffold(
        topBar = {
            AppHeader(
                activeTab = ActiveTab.ALBUMS,
                username = viewModel.username,
                navigation = HeaderNavigation(
                    onGalleryClick = onGalleryClick,
                    onAlbumsClick = { /* already on albums */ },
                    onSearchClick = onSearchClick,
                    onTrashClick = onTrashClick,
                    onSettingsClick = onSettingsClick,
                    onLogout = { viewModel.logout(onLogout) }
                )
            )
        }
    ) { padding ->
        Column(modifier = Modifier.padding(padding)) {
            // ── Page header with inline "New Album" button ───────────
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Text(
                    "Albums",
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold
                )
                Button(
                    onClick = { viewModel.showCreateDialog = true },
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
                ) {
                    Icon(
                        Icons.Default.Add,
                        contentDescription = null,
                        modifier = Modifier.size(18.dp)
                    )
                    Spacer(Modifier.width(6.dp))
                    Text("New Album")
                }
            }

            viewModel.error?.let { err ->
                Text(
                    err,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(horizontal = 16.dp),
                    style = MaterialTheme.typography.bodySmall
                )
            }

            if (albums.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Text(
                        "No albums yet.\nTap New Album to create one.",
                        textAlign = TextAlign.Center,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            } else {
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(150.dp),
                    contentPadding = PaddingValues(16.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    items(albums, key = { it.localId }) { album ->
                        AlbumCard(
                            album = album,
                            onClick = { onAlbumClick(album.localId) }
                        )
                    }
                }
            }
        }
    }

    // Create album dialog
    if (viewModel.showCreateDialog) {
        AlertDialog(
            onDismissRequest = { viewModel.showCreateDialog = false },
            title = { Text("New Album") },
            text = {
                OutlinedTextField(
                    value = viewModel.newAlbumName,
                    onValueChange = { viewModel.newAlbumName = it },
                    label = { Text("Album name") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
            },
            confirmButton = {
                TextButton(
                    onClick = { viewModel.createAlbum() },
                    enabled = viewModel.newAlbumName.isNotBlank()
                ) {
                    Text("Create")
                }
            },
            dismissButton = {
                TextButton(onClick = { viewModel.showCreateDialog = false }) {
                    Text("Cancel")
                }
            }
        )
    }
}

@Composable
private fun AlbumCard(album: AlbumEntity, onClick: () -> Unit = {}) {
    Card(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick)
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Surface(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(1f),
                color = MaterialTheme.colorScheme.surfaceVariant,
                shape = MaterialTheme.shapes.small
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(
                        album.name.take(2).uppercase(),
                        style = MaterialTheme.typography.headlineMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            Spacer(Modifier.height(8.dp))
            Text(
                album.name,
                style = MaterialTheme.typography.titleSmall,
                maxLines = 1
            )
        }
    }
}
