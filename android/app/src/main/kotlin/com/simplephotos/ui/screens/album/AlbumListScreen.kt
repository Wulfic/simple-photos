/**
 * Composable screen that displays the user's albums in a scrollable list
 * with options to create, rename, and delete albums. Shared albums are
 * rendered inline at the bottom — matching the web Albums page layout.
 */
package com.simplephotos.ui.screens.album

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Person
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SharedAlbumInfo
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.data.repository.SharingRepository
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

/**
 * ViewModel for the album list screen. Loads user albums, smart album counts,
 * shared albums, syncs album manifests from the server, and manages album CRUD.
 */
@HiltViewModel
class AlbumViewModel @Inject constructor(
    private val albumRepository: AlbumRepository,
    private val authRepository: AuthRepository,
    private val photoRepository: PhotoRepository,
    private val sharingRepository: SharingRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    val albums = albumRepository.getAllAlbums()
    var error by mutableStateOf<String?>(null)
    var showCreateDialog by mutableStateOf(false)
    var newAlbumName by mutableStateOf("")
    var username by mutableStateOf("")
        private set

    /** Map of albumId -> first PhotoEntity (for cover image preview) */
    var albumCoverPhotos by mutableStateOf<Map<String, PhotoEntity>>(emptyMap())
        private set

    /** Base URL for server-based thumbnails */
    var serverBaseUrl by mutableStateOf("")
        private set

    var encryptionMode by mutableStateOf("plain")
        private set

    /** Photo counts for smart/default albums */
    var totalCount by mutableStateOf(0)
        private set
    var favoritesCount by mutableStateOf(0)
        private set
    var photosCount by mutableStateOf(0)
        private set
    var gifsCount by mutableStateOf(0)
        private set
    var videosCount by mutableStateOf(0)
        private set
    var audioCount by mutableStateOf(0)
        private set

    /** Cover photos for smart albums (keyed by smart album ID) */
    var smartAlbumCoverPhotos by mutableStateOf<Map<String, PhotoEntity>>(emptyMap())
        private set

    // ── Shared albums ────────────────────────────────────────────────────
    var sharedAlbums by mutableStateOf<List<SharedAlbumInfo>>(emptyList())
        private set
    var sharedLoading by mutableStateOf(true)
        private set
    var showCreateSharedDialog by mutableStateOf(false)
    var newSharedAlbumName by mutableStateOf("")

    init {
        viewModelScope.launch {
            try {
                val prefs = dataStore.data.first()
                username = prefs[KEY_USERNAME] ?: ""
            } catch (_: Exception) {}
            // Load server config
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                encryptionMode = withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() }
            } catch (_: Exception) {}
            // Sync album manifests from server (picks up web-created albums)
            try {
                withContext(Dispatchers.IO) { albumRepository.syncAlbumsFromServer() }
            } catch (_: Exception) {}
            // Load photo counts for smart albums
            loadSmartAlbumCounts()
            // Load shared albums (displayed at the bottom of the albums page)
            loadSharedAlbums()
        }
    }

    private fun loadSmartAlbumCounts() {
        viewModelScope.launch {
            try {
                val allPhotos = withContext(Dispatchers.IO) {
                    photoRepository.getAllPhotos().first()
                }
                totalCount = allPhotos.size
                favoritesCount = allPhotos.count { it.isFavorite }
                photosCount = allPhotos.count { it.mediaType == "photo" || it.mediaType == "gif" }
                gifsCount = allPhotos.count { it.mediaType == "gif" }
                videosCount = allPhotos.count { it.mediaType == "video" }
                audioCount = allPhotos.count { it.mediaType == "audio" }

                // Load cover photos for smart albums (most recent photo matching each filter)
                val sorted = allPhotos.sortedByDescending { it.takenAt }
                val covers = mutableMapOf<String, PhotoEntity>()
                sorted.firstOrNull { it.isFavorite }?.let { covers["smart-favorites"] = it }
                sorted.firstOrNull { it.mediaType == "photo" || it.mediaType == "gif" }?.let { covers["smart-photos"] = it }
                sorted.firstOrNull { it.mediaType == "gif" }?.let { covers["smart-gifs"] = it }
                sorted.firstOrNull { it.mediaType == "video" }?.let { covers["smart-videos"] = it }
                sorted.firstOrNull { it.mediaType == "audio" }?.let { covers["smart-audio"] = it }
                smartAlbumCoverPhotos = covers
            } catch (_: Exception) {}
        }
    }

    /** Load cover photo for each album (call whenever albums list updates). */
    fun loadCoverPhotos(albums: List<AlbumEntity>) {
        viewModelScope.launch {
            val covers = mutableMapOf<String, PhotoEntity>()
            for (album in albums) {
                try {
                    val photoIds = withContext(Dispatchers.IO) { albumRepository.getPhotoIdsForAlbum(album.localId) }
                    val firstId = photoIds.firstOrNull() ?: continue
                    val photo = withContext(Dispatchers.IO) { photoRepository.getPhoto(firstId) }
                    if (photo != null) covers[album.localId] = photo
                } catch (_: Exception) {}
            }
            albumCoverPhotos = covers
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

    // ── Shared album operations ──────────────────────────────────────────

    /** Fetch shared albums from the server. */
    fun loadSharedAlbums() {
        viewModelScope.launch {
            sharedLoading = true
            try {
                sharedAlbums = withContext(Dispatchers.IO) { sharingRepository.listAlbums() }
            } catch (_: Exception) {
                // Non-fatal: shared albums may not be available
            } finally {
                sharedLoading = false
            }
        }
    }

    /** Create a new shared album and refresh the list. */
    fun createSharedAlbum() {
        val name = newSharedAlbumName.trim()
        if (name.isBlank()) return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { sharingRepository.createAlbum(name) }
                newSharedAlbumName = ""
                showCreateSharedDialog = false
                loadSharedAlbums()
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
    onSecureGalleryClick: () -> Unit = {},
    onSharedAlbumsClick: () -> Unit = {},
    onDiagnosticsClick: () -> Unit = {},
    onLogout: () -> Unit,
    onAlbumClick: (String) -> Unit = {},
    /** Navigate into the dedicated Shared Albums screen (e.g. to view detail) */
    onSharedAlbumClick: () -> Unit = {},
    isAdmin: Boolean = false,
    viewModel: AlbumViewModel = hiltViewModel()
) {
    val albums by viewModel.albums.collectAsState(initial = emptyList())
    val isSystemDark = androidx.compose.foundation.isSystemInDarkTheme()

    // Load cover photos whenever the albums list changes
    LaunchedEffect(albums) {
        viewModel.loadCoverPhotos(albums)
    }

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
                .verticalScroll(rememberScrollState())
        ) {
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

            // Build combined list: smart albums pinned at top, then user albums
            val smartAlbumEntries = buildList {
                add(Triple("smart-favorites", "Favorites", viewModel.favoritesCount))
                add(Triple("smart-photos", "Photos", viewModel.photosCount))
                add(Triple("smart-gifs", "GIFs", viewModel.gifsCount))
                add(Triple("smart-videos", "Videos", viewModel.videosCount))
                add(Triple("smart-audio", "Audio", viewModel.audioCount))
            }

            // Render all cards (smart + user) in 2-column rows
            val allItems = smartAlbumEntries.map { (id, label, count) ->
                AlbumGridItem.Smart(id, label, count)
            } + albums.map { AlbumGridItem.UserAlbum(it) }

            allItems.chunked(2).forEach { rowItems ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    rowItems.forEach { item ->
                        Box(modifier = Modifier.weight(1f)) {
                            when (item) {
                                is AlbumGridItem.Smart -> {
                                    SmartAlbumCard(
                                        label = item.label,
                                        count = item.count,
                                        coverPhoto = viewModel.smartAlbumCoverPhotos[item.id],
                                        serverBaseUrl = viewModel.serverBaseUrl,
                                        encryptionMode = viewModel.encryptionMode,
                                        onClick = { onAlbumClick(item.id) }
                                    )
                                }
                                is AlbumGridItem.UserAlbum -> {
                                    AlbumCard(
                                        album = item.album,
                                        coverPhoto = viewModel.albumCoverPhotos[item.album.localId],
                                        serverBaseUrl = viewModel.serverBaseUrl,
                                        encryptionMode = viewModel.encryptionMode,
                                        onClick = { onAlbumClick(item.album.localId) }
                                    )
                                }
                            }
                        }
                    }
                    if (rowItems.size == 1) {
                        Spacer(Modifier.weight(1f))
                    }
                }
            }

            if (allItems.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(vertical = 48.dp),
                    contentAlignment = Alignment.Center
                ) {
                    Text(
                        "No albums yet.\nTap New Album to create one.",
                        textAlign = TextAlign.Center,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            // ── Shared Albums section (mirrors the web Albums page layout) ──
            Spacer(Modifier.height(24.dp))
            HorizontalDivider(
                modifier = Modifier.padding(horizontal = 16.dp),
                color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f)
            )
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Text(
                    "Shared Albums",
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold
                )
                Button(
                    onClick = { viewModel.showCreateSharedDialog = true },
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
                ) {
                    Icon(
                        Icons.Default.Add,
                        contentDescription = null,
                        modifier = Modifier.size(18.dp)
                    )
                    Spacer(Modifier.width(6.dp))
                    Text("New")
                }
            }

            if (viewModel.sharedLoading && viewModel.sharedAlbums.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(vertical = 32.dp),
                    contentAlignment = Alignment.Center
                ) {
                    CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
                }
            } else if (viewModel.sharedAlbums.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(vertical = 32.dp),
                    contentAlignment = Alignment.Center
                ) {
                    Text(
                        "No shared albums yet.\nCreate one to share photos with other users.",
                        textAlign = TextAlign.Center,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            } else {
                // Render shared album cards in 2-column rows
                viewModel.sharedAlbums.chunked(2).forEach { rowItems ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 4.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        rowItems.forEach { album ->
                            Box(modifier = Modifier.weight(1f)) {
                                SharedAlbumCard(
                                    album = album,
                                    onClick = { onSharedAlbumClick() }
                                )
                            }
                        }
                        if (rowItems.size == 1) {
                            Spacer(Modifier.weight(1f))
                        }
                    }
                }
            }

            Spacer(Modifier.height(16.dp))
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
                    onValueChange = { if (it.length <= 200) viewModel.newAlbumName = it },
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

    // Create shared album dialog
    if (viewModel.showCreateSharedDialog) {
        AlertDialog(
            onDismissRequest = { viewModel.showCreateSharedDialog = false },
            title = { Text("New Shared Album") },
            text = {
                OutlinedTextField(
                    value = viewModel.newSharedAlbumName,
                    onValueChange = { if (it.length <= 200) viewModel.newSharedAlbumName = it },
                    label = { Text("Album name") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
            },
            confirmButton = {
                TextButton(
                    onClick = { viewModel.createSharedAlbum() },
                    enabled = viewModel.newSharedAlbumName.isNotBlank()
                ) {
                    Text("Create")
                }
            },
            dismissButton = {
                TextButton(onClick = { viewModel.showCreateSharedDialog = false }) {
                    Text("Cancel")
                }
            }
        )
    }
}

@Composable
private fun AlbumCard(
    album: AlbumEntity,
    coverPhoto: PhotoEntity? = null,
    serverBaseUrl: String = "",
    encryptionMode: String = "plain",
    onClick: () -> Unit = {}
) {
    val context = LocalContext.current
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
                if (coverPhoto != null) {
                    // Determine thumbnail source
                    val thumbModel: Any? = when {
                        encryptionMode == "plain" && coverPhoto.serverPhotoId != null ->
                            "$serverBaseUrl/api/photos/${coverPhoto.serverPhotoId}/thumb"
                        coverPhoto.thumbnailPath != null -> java.io.File(coverPhoto.thumbnailPath!!)
                        coverPhoto.localPath != null -> coverPhoto.localPath
                        else -> null
                    }
                    if (thumbModel != null) {
                        AsyncImage(
                            model = ImageRequest.Builder(context)
                                .data(thumbModel)
                                .crossfade(true)
                                .build(),
                            contentDescription = album.name,
                            contentScale = ContentScale.Crop,
                            modifier = Modifier
                                .fillMaxSize()
                                .clip(MaterialTheme.shapes.small)
                        )
                    } else {
                        Box(contentAlignment = Alignment.Center) {
                            Text(
                                album.name.take(2).uppercase(),
                                style = MaterialTheme.typography.headlineMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    }
                } else {
                    Box(contentAlignment = Alignment.Center) {
                        Text(
                            album.name.take(2).uppercase(),
                            style = MaterialTheme.typography.headlineMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
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

// ── Smart/Default Album support ─────────────────────────────────────

/** Sealed class for unified rendering of smart and user albums in one grid. */
private sealed class AlbumGridItem {
    data class Smart(val id: String, val label: String, val count: Int) : AlbumGridItem()
    data class UserAlbum(val album: AlbumEntity) : AlbumGridItem()
}

/** Renders a smart album card with cover photo thumbnail (mirrors AlbumCard style). */
@Composable
private fun SmartAlbumCard(
    label: String,
    count: Int,
    coverPhoto: PhotoEntity? = null,
    serverBaseUrl: String = "",
    encryptionMode: String = "plain",
    onClick: () -> Unit = {}
) {
    val context = LocalContext.current
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        shape = MaterialTheme.shapes.medium
    ) {
        Column {
            // Thumbnail area (same aspect ratio as album cover)
            Surface(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(1f),
                color = MaterialTheme.colorScheme.surfaceVariant,
                shape = MaterialTheme.shapes.small
            ) {
                if (coverPhoto != null) {
                    val thumbModel: Any? = when {
                        encryptionMode == "plain" && coverPhoto.serverPhotoId != null ->
                            "$serverBaseUrl/api/photos/${coverPhoto.serverPhotoId}/thumb"
                        coverPhoto.thumbnailPath != null -> java.io.File(coverPhoto.thumbnailPath!!)
                        coverPhoto.localPath != null -> coverPhoto.localPath
                        else -> null
                    }
                    if (thumbModel != null) {
                        AsyncImage(
                            model = ImageRequest.Builder(context)
                                .data(thumbModel)
                                .crossfade(true)
                                .build(),
                            contentDescription = label,
                            contentScale = ContentScale.Crop,
                            modifier = Modifier
                                .fillMaxSize()
                                .clip(MaterialTheme.shapes.small)
                        )
                    } else {
                        Box(contentAlignment = Alignment.Center) {
                            Text(
                                "$count",
                                style = MaterialTheme.typography.headlineMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    }
                } else {
                    Box(contentAlignment = Alignment.Center) {
                        Text(
                            "$count",
                            style = MaterialTheme.typography.headlineMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
            Column(modifier = Modifier.padding(8.dp)) {
                Text(
                    label,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1
                )
                Text(
                    "$count items",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

// ── Shared Album card (matches the web's SharedAlbum grid card style) ────────

/**
 * Renders a shared album card in the 2-column grid. Shows photo count,
 * member count, album name, and owner — matching the web Albums page layout.
 */
@Composable
private fun SharedAlbumCard(
    album: SharedAlbumInfo,
    onClick: () -> Unit = {}
) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        shape = MaterialTheme.shapes.medium
    ) {
        Column {
            // Cover area — gradient placeholder with photo/member counts
            Surface(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(1f),
                color = Color.Transparent,
                shape = MaterialTheme.shapes.small
            ) {
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .background(
                            Brush.linearGradient(
                                listOf(
                                    Color(0xFF10B981).copy(alpha = 0.15f),
                                    Color(0xFF3B82F6).copy(alpha = 0.15f)
                                )
                            )
                        ),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            "${album.photoCount}",
                            style = MaterialTheme.typography.headlineMedium,
                            fontWeight = FontWeight.SemiBold,
                            color = Color(0xFF10B981)
                        )
                        Spacer(Modifier.height(2.dp))
                        Text(
                            "${album.memberCount} member${if (album.memberCount != 1L) "s" else ""}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
            Column(modifier = Modifier.padding(8.dp)) {
                Text(
                    album.name,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1
                )
                Text(
                    if (album.isOwner) "You" else album.ownerUsername,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}
