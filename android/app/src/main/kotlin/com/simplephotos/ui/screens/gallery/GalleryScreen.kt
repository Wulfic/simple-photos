package com.simplephotos.ui.screens.gallery

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items as lazyItems
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.material3.pulltorefresh.PullToRefreshContainer
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.sync.DiagnosticLogger
import com.simplephotos.sync.SyncScheduler
import com.simplephotos.ui.components.ActiveTab
import com.simplephotos.ui.components.AppHeader
import com.simplephotos.ui.components.HeaderNavigation
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_DIAGNOSTIC_LOGGING
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import com.simplephotos.ui.theme.ThemeState
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.*
import javax.inject.Inject
import com.simplephotos.R
import androidx.compose.ui.res.painterResource

// ── Day grouping helper ─────────────────────────────────────────────────────

private fun groupPhotosByDay(photos: List<PhotoEntity>): List<Pair<String, List<PhotoEntity>>> {
    val fmt = SimpleDateFormat("EEEE, MMMM d, yyyy", Locale.getDefault())
    return photos
        .sortedByDescending { it.takenAt }
        .groupBy { fmt.format(Date(it.takenAt)) }
        .toList()
}

// Sealed class for grid items (headers vs photos)
private sealed class GalleryGridItem {
    data class Header(val dateLabel: String, val photoIds: Set<String>) : GalleryGridItem()
    data class Photo(val photo: PhotoEntity) : GalleryGridItem()
}

private fun buildGridItems(dayGroups: List<Pair<String, List<PhotoEntity>>>): List<GalleryGridItem> {
    val items = mutableListOf<GalleryGridItem>()
    for ((dateLabel, photos) in dayGroups) {
        items.add(GalleryGridItem.Header(dateLabel, photos.map { it.localId }.toSet()))
        for (photo in photos) {
            items.add(GalleryGridItem.Photo(photo))
        }
    }
    return items
}

// ── ViewModel ───────────────────────────────────────────────────────────────

@HiltViewModel
class GalleryViewModel @Inject constructor(
    private val photoRepository: PhotoRepository,
    private val authRepository: AuthRepository,
    private val api: ApiService,
    private val db: AppDatabase,
    private val albumRepository: AlbumRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    val photos = photoRepository.getAllPhotos()
    var error by mutableStateOf<String?>(null)
    var isSyncing by mutableStateOf(false)
        private set
    var isImporting by mutableStateOf(false)
        private set
    var lastSyncResult by mutableStateOf<String?>(null)
        private set

    var serverBaseUrl by mutableStateOf("")
        private set
    var encryptionMode by mutableStateOf("plain")
        private set
    var username by mutableStateOf("")
        private set

    // ── Multi-select state ────────────────────────────────────────
    var selectedIds by mutableStateOf(emptySet<String>())
        private set
    var isSelectionMode by mutableStateOf(false)
        private set

    // ── Album state for picker ────────────────────────────────────
    val albums = albumRepository.getAllAlbums()

    init {
        viewModelScope.launch {
            try {
                val url = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val mode = withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() }
                val prefs = dataStore.data.first()
                serverBaseUrl = url
                encryptionMode = mode
                username = prefs[KEY_USERNAME] ?: ""
            } catch (e: Exception) {
                error = "Init failed: ${e.message}"
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

    fun selectAll(allPhotos: List<PhotoEntity>) {
        isSelectionMode = true
        selectedIds = allPhotos.map { it.localId }.toSet()
    }

    fun selectDay(dayPhotoIds: Set<String>) {
        isSelectionMode = true
        selectedIds = selectedIds + dayPhotoIds
    }

    fun clearSelection() {
        selectedIds = emptySet()
        isSelectionMode = false
    }

    fun deleteSelectedPhotos(allPhotos: List<PhotoEntity>) {
        viewModelScope.launch {
            try {
                val toDelete = allPhotos.filter { it.localId in selectedIds }
                for (p in toDelete) {
                    withContext(Dispatchers.IO) { photoRepository.deletePhoto(p) }
                }
                clearSelection()
            } catch (e: Exception) {
                error = "Delete failed: ${e.message}"
            }
        }
    }

    fun addSelectedToAlbum(albumId: String) {
        viewModelScope.launch {
            try {
                for (id in selectedIds) {
                    withContext(Dispatchers.IO) { albumRepository.addPhotoToAlbum(id, albumId) }
                }
                clearSelection()
            } catch (e: Exception) {
                error = "Add to album failed: ${e.message}"
            }
        }
    }

    fun createAlbumAndAddSelected(name: String) {
        viewModelScope.launch {
            try {
                val album = withContext(Dispatchers.IO) { albumRepository.createAlbum(name) }
                for (id in selectedIds) {
                    withContext(Dispatchers.IO) { albumRepository.addPhotoToAlbum(id, album.localId) }
                }
                clearSelection()
            } catch (e: Exception) {
                error = "Create album failed: ${e.message}"
            }
        }
    }

    fun sendDiagnosticReport(context: android.content.Context) {
        viewModelScope.launch {
            try {
                val prefs = dataStore.data.first()
                val loggingEnabled = prefs[KEY_DIAGNOSTIC_LOGGING] ?: false
                val diag = DiagnosticLogger(api, loggingEnabled)
                diag.info("AppDiagnostic", "Gallery opened — sending diagnostic report")

                val pendingCount = withContext(Dispatchers.IO) { db.photoDao().getByStatus(SyncStatus.PENDING).size }
                val failedCount = withContext(Dispatchers.IO) { db.photoDao().getByStatus(SyncStatus.FAILED).size }
                val uploadingCount = withContext(Dispatchers.IO) { db.photoDao().getByStatus(SyncStatus.UPLOADING).size }
                val syncedCount = withContext(Dispatchers.IO) { db.photoDao().getByStatus(SyncStatus.SYNCED).size }
                diag.info("AppDiagnostic", "Local DB photo status", mapOf(
                    "pending" to pendingCount.toString(), "failed" to failedCount.toString(),
                    "uploading" to uploadingCount.toString(), "synced" to syncedCount.toString()
                ))

                val mode = try { withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() } } catch (e: Exception) { "error: ${e.message}" }
                diag.info("AppDiagnostic", "Encryption mode: $mode")

                val enabledFolders = withContext(Dispatchers.IO) { db.backupFolderDao().getEnabledFolders() }
                diag.info("AppDiagnostic", "Backup folders", mapOf(
                    "enabledCount" to enabledFolders.size.toString(),
                    "folders" to enabledFolders.joinToString(", ") { "${it.bucketName}(${it.relativePath}, enabled=${it.enabled})" }
                ))

                try {
                    val wm = androidx.work.WorkManager.getInstance(context)
                    val periodicInfos = wm.getWorkInfosForUniqueWork("photo_backup").get()
                    val reactiveInfos = wm.getWorkInfosForUniqueWork("photo_backup_reactive").get()
                    diag.info("AppDiagnostic", "WorkManager periodic tasks", mapOf(
                        "count" to periodicInfos.size.toString(),
                        "states" to periodicInfos.joinToString(", ") { "${it.state}(attempt=${it.runAttemptCount})" }
                    ))
                    diag.info("AppDiagnostic", "WorkManager reactive tasks", mapOf(
                        "count" to reactiveInfos.size.toString(),
                        "states" to reactiveInfos.joinToString(", ") { "${it.state}(attempt=${it.runAttemptCount})" }
                    ))
                } catch (e: Exception) {
                    diag.error("AppDiagnostic", "Failed to query WorkManager: ${e.message}")
                }

                try {
                    val health = withContext(Dispatchers.IO) { api.health() }
                    diag.info("AppDiagnostic", "Server health OK", health.mapValues { it.value })
                } catch (e: Exception) {
                    diag.error("AppDiagnostic", "Server health check failed: ${e.message}", mapOf("exception" to (e::class.simpleName ?: "Unknown")))
                }

                withContext(Dispatchers.IO) { diag.flush() }
            } catch (e: Exception) {
                android.util.Log.e("AppDiagnostic", "Failed to send diagnostic report", e)
            }
        }
    }

    fun logout(onLoggedOut: () -> Unit) {
        viewModelScope.launch {
            try { withContext(Dispatchers.IO) { authRepository.logout() } ; onLoggedOut() } catch (_: Exception) { onLoggedOut() }
        }
    }

    fun deletePhoto(photo: PhotoEntity) {
        viewModelScope.launch {
            try { withContext(Dispatchers.IO) { photoRepository.deletePhoto(photo) } } catch (e: Exception) { error = e.message }
        }
    }

    fun syncFromServer() {
        if (isSyncing) return
        viewModelScope.launch {
            isSyncing = true; error = null; lastSyncResult = null
            try {
                val url = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val mode = withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() }
                serverBaseUrl = url; encryptionMode = mode
                val imported = withContext(Dispatchers.IO) { photoRepository.syncFromServer() }
                lastSyncResult = if (imported > 0) "Synced $imported new items" else "Up to date"
            } catch (e: Exception) { error = "Sync failed: ${e.message}" } finally { isSyncing = false }
        }
    }

    fun importFromUris(uris: List<Uri>, context: android.content.Context) {
        if (uris.isEmpty()) return
        viewModelScope.launch {
            isImporting = true; error = null; var count = 0
            for (uri in uris) {
                try {
                    val resolver = context.contentResolver
                    val mimeType = resolver.getType(uri) ?: "image/jpeg"
                    val mediaType = when {
                        mimeType.startsWith("video/") -> "video"
                        mimeType == "image/gif" -> "gif"
                        else -> "photo"
                    }
                    val data = withContext(Dispatchers.IO) { resolver.openInputStream(uri)?.use { it.readBytes() } } ?: continue
                    val thumbBytes = if (mediaType != "video") {
                        withContext(Dispatchers.IO) {
                            val opts = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }
                            android.graphics.BitmapFactory.decodeByteArray(data, 0, data.size, opts)
                            val scaleFactor = maxOf(opts.outWidth, opts.outHeight) / 256
                            val opts2 = android.graphics.BitmapFactory.Options().apply { inSampleSize = maxOf(1, scaleFactor) }
                            val bitmap = android.graphics.BitmapFactory.decodeByteArray(data, 0, data.size, opts2)
                            val stream = java.io.ByteArrayOutputStream()
                            bitmap?.compress(android.graphics.Bitmap.CompressFormat.JPEG, 80, stream)
                            bitmap?.recycle()
                            stream.toByteArray()
                        }
                    } else { ByteArray(0) }

                    val localId = java.util.UUID.randomUUID().toString()
                    val filename = uri.lastPathSegment ?: "import_$localId"
                    val photo = PhotoEntity(
                        localId = localId, filename = filename, takenAt = System.currentTimeMillis(),
                        mimeType = mimeType, mediaType = mediaType, width = 0, height = 0,
                        localPath = uri.toString(), syncStatus = SyncStatus.PENDING
                    )
                    photoRepository.insertPhoto(photo)
                    if (thumbBytes.isNotEmpty()) {
                        val thumbPath = photoRepository.saveThumbnailToDisk(localId, thumbBytes)
                        photoRepository.insertPhoto(photo.copy(thumbnailPath = thumbPath))
                    }
                    try {
                        withContext(Dispatchers.IO) {
                            if (encryptionMode == "plain") photoRepository.uploadPhotoPlain(photo, data)
                            else photoRepository.uploadPhoto(photo, data, if (thumbBytes.isEmpty()) data else thumbBytes)
                        }
                    } catch (_: Exception) {}
                    count++
                } catch (e: Exception) { error = "Import error: ${e.message}" }
            }
            lastSyncResult = "Imported $count photo${if (count != 1) "s" else ""}"
            isImporting = false
        }
    }
}

// ── Screen ──────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun GalleryScreen(
    onPhotoClick: (String) -> Unit,
    onAlbumsClick: () -> Unit,
    onSearchClick: () -> Unit,
    onTrashClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onLogout: () -> Unit,
    viewModel: GalleryViewModel = hiltViewModel()
) {
    val photos by viewModel.photos.collectAsState(initial = emptyList())
    val albums by viewModel.albums.collectAsState(initial = emptyList())
    val context = LocalContext.current
    var showAlbumPicker by remember { mutableStateOf(false) }

    // Album filter state
    var albumFilter by remember { mutableStateOf("all") }

    // Apply album filter
    val filteredPhotos = remember(photos, albumFilter) {
        when (albumFilter) {
            "favorites" -> photos.filter { it.isFavorite }
            "photos" -> photos.filter { it.mediaType == "photo" || it.mediaType == "gif" }
            "gifs" -> photos.filter { it.mediaType == "gif" }
            "videos" -> photos.filter { it.mediaType == "video" }
            else -> photos
        }
    }

    // Build day-grouped grid items from filtered photos
    val gridItems = remember(filteredPhotos) { buildGridItems(groupPhotosByDay(filteredPhotos)) }

    val pickMediaLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenMultipleDocuments()
    ) { uris: List<Uri> -> viewModel.importFromUris(uris, context) }

    LaunchedEffect(Unit) { try { SyncScheduler.schedule(context) } catch (_: Exception) {} }
    LaunchedEffect(Unit) { viewModel.syncFromServer(); try { SyncScheduler.triggerNow(context) } catch (_: Exception) {} }
    LaunchedEffect(Unit) { viewModel.sendDiagnosticReport(context) }

    Scaffold(
        topBar = {
            if (viewModel.isSelectionMode) {
                Surface(
                    modifier = Modifier.fillMaxWidth(),
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    shadowElevation = 2.dp
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .statusBarsPadding()
                            .padding(horizontal = 12.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            IconButton(onClick = { viewModel.clearSelection() }, modifier = Modifier.size(32.dp)) {
                                Icon(Icons.Default.Close, contentDescription = "Cancel", modifier = Modifier.size(20.dp))
                            }
                            Spacer(Modifier.width(4.dp))
                            Text(
                                "${viewModel.selectedIds.size} selected",
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = FontWeight.Medium
                            )
                            Spacer(Modifier.width(8.dp))
                            TextButton(
                                onClick = { viewModel.selectAll(photos) },
                                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp)
                            ) {
                                Text("Select All", fontSize = 12.sp)
                            }
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            OutlinedButton(
                                onClick = { showAlbumPicker = true },
                                enabled = viewModel.selectedIds.isNotEmpty(),
                                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp)
                            ) {
                                Icon(painter = painterResource(R.drawable.ic_folder), contentDescription = null, modifier = Modifier.size(16.dp))
                                Spacer(Modifier.width(4.dp))
                                Text("Album", fontSize = 12.sp)
                            }
                            Button(
                                onClick = { viewModel.deleteSelectedPhotos(photos) },
                                enabled = viewModel.selectedIds.isNotEmpty(),
                                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp)
                            ) {
                                Icon(painter = painterResource(R.drawable.ic_trashcan), contentDescription = null, modifier = Modifier.size(16.dp))
                                Spacer(Modifier.width(4.dp))
                                Text("Delete", fontSize = 12.sp)
                            }
                        }
                    }
                }
            } else {
                AppHeader(
                    activeTab = ActiveTab.GALLERY,
                    username = viewModel.username,
                    navigation = HeaderNavigation(
                        onGalleryClick = { /* already on gallery */ },
                        onAlbumsClick = onAlbumsClick,
                        onSearchClick = onSearchClick,
                        onTrashClick = onTrashClick,
                        onSettingsClick = onSettingsClick,
                        onLogout = { viewModel.logout(onLogout) }
                    ),
                    isSyncing = viewModel.isSyncing,
                    syncLabel = if (viewModel.isSyncing) "Syncing" else null
                )
            }
        },
        floatingActionButton = {
            if (!viewModel.isSelectionMode) {
                FloatingActionButton(
                    onClick = { pickMediaLauncher.launch(arrayOf("image/*", "video/*")) },
                    containerColor = MaterialTheme.colorScheme.primary
                ) {
                    if (viewModel.isImporting) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(24.dp), strokeWidth = 2.dp,
                            color = MaterialTheme.colorScheme.onPrimary
                        )
                    } else {
                        Icon(Icons.Default.Add, contentDescription = "Add photos", tint = MaterialTheme.colorScheme.onPrimary)
                    }
                }
            }
        }
    ) { padding ->
        val pullToRefreshState = rememberPullToRefreshState()
        if (pullToRefreshState.isRefreshing) {
            LaunchedEffect(true) {
                viewModel.syncFromServer()
                pullToRefreshState.endRefresh()
            }
        }
        Box(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
                .nestedScroll(pullToRefreshState.nestedScrollConnection)
        ) {
            Column(modifier = Modifier.fillMaxSize()) {
                viewModel.lastSyncResult?.let { msg ->
                    Text(msg, color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp), style = MaterialTheme.typography.bodySmall)
                }
                viewModel.error?.let { err ->
                    Text(err, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp), style = MaterialTheme.typography.bodySmall)
                }

                // Album filter tabs (plain mode only)
                if (viewModel.encryptionMode == "plain") {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 8.dp, vertical = 6.dp),
                        horizontalArrangement = Arrangement.spacedBy(6.dp)
                    ) {
                        listOf("all" to "All", "favorites" to "★ Favorites", "photos" to "Photos", "gifs" to "GIFs", "videos" to "Videos").forEach { (key, label) ->
                            FilterChip(
                                selected = albumFilter == key,
                                onClick = { albumFilter = key },
                                label = { Text(label, fontSize = 12.sp) },
                                colors = FilterChipDefaults.filterChipColors(
                                    selectedContainerColor = MaterialTheme.colorScheme.primary,
                                    selectedLabelColor = MaterialTheme.colorScheme.onPrimary
                                ),
                                modifier = Modifier.height(30.dp)
                            )
                        }
                    }
                }

                if (filteredPhotos.isEmpty() && !viewModel.isSyncing) {
                    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text("No photos yet", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Spacer(Modifier.height(8.dp))
                            Text("Tap + to add photos or grant permissions for auto-backup", textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(horizontal = 32.dp))
                        }
                    }
                } else if (filteredPhotos.isEmpty() && viewModel.isSyncing) {
                    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            CircularProgressIndicator()
                            Spacer(Modifier.height(16.dp))
                            Text("Syncing from server...")
                        }
                    }
                } else {
                    // Day-grouped photo grid
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(100.dp),
                        contentPadding = PaddingValues(2.dp),
                        horizontalArrangement = Arrangement.spacedBy(2.dp),
                        verticalArrangement = Arrangement.spacedBy(2.dp)
                    ) {
                        for (item in gridItems) {
                            when (item) {
                                is GalleryGridItem.Header -> {
                                    item(
                                        span = { GridItemSpan(maxLineSpan) },
                                        key = "header_${item.dateLabel}"
                                    ) {
                                        DayHeader(
                                            dateLabel = item.dateLabel,
                                            isSelectionMode = viewModel.isSelectionMode,
                                            allSelected = item.photoIds.all { it in viewModel.selectedIds },
                                            onSelectDay = { viewModel.selectDay(item.photoIds) }
                                        )
                                    }
                                }
                                is GalleryGridItem.Photo -> {
                                    item(key = item.photo.localId) {
                                        MediaTile(
                                            photo = item.photo,
                                            serverBaseUrl = viewModel.serverBaseUrl,
                                            encryptionMode = viewModel.encryptionMode,
                                            isSelectionMode = viewModel.isSelectionMode,
                                            isSelected = item.photo.localId in viewModel.selectedIds,
                                            onTap = {
                                                if (viewModel.isSelectionMode) viewModel.toggleSelect(item.photo.localId)
                                                else onPhotoClick(item.photo.localId)
                                            },
                                            onLongPress = { viewModel.enterSelectionMode(item.photo.localId) }
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
            PullToRefreshContainer(state = pullToRefreshState, modifier = Modifier.align(Alignment.TopCenter))
        }
    }

    // ── Album Picker Dialog ─────────────────────────────────────────────────
    if (showAlbumPicker) {
        AlbumPickerDialog(
            albums = albums,
            onDismiss = { showAlbumPicker = false },
            onAlbumSelected = { albumId ->
                viewModel.addSelectedToAlbum(albumId)
                showAlbumPicker = false
            },
            onCreateAlbum = { name ->
                viewModel.createAlbumAndAddSelected(name)
                showAlbumPicker = false
            }
        )
    }
}

// ── Day Header ──────────────────────────────────────────────────────────────

@Composable
private fun DayHeader(
    dateLabel: String,
    isSelectionMode: Boolean,
    allSelected: Boolean,
    onSelectDay: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(
            dateLabel,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.weight(1f)
        )
        // Day select bubble
        if (isSelectionMode) {
            Surface(
                modifier = Modifier
                    .clip(RoundedCornerShape(16.dp))
                    .clickable(onClick = onSelectDay),
                color = if (allSelected)
                    Color(0xFF22C55E).copy(alpha = 0.15f)
                else
                    MaterialTheme.colorScheme.surfaceVariant,
                shape = RoundedCornerShape(16.dp)
            ) {
                Text(
                    if (allSelected) "Selected" else "Select day",
                    fontSize = 11.sp,
                    fontWeight = FontWeight.Medium,
                    color = if (allSelected) Color(0xFF22C55E) else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp)
                )
            }
        }
    }
}

// ── Album Picker Dialog ─────────────────────────────────────────────────────

@Composable
private fun AlbumPickerDialog(
    albums: List<AlbumEntity>,
    onDismiss: () -> Unit,
    onAlbumSelected: (String) -> Unit,
    onCreateAlbum: (String) -> Unit
) {
    var showCreateField by remember { mutableStateOf(false) }
    var newAlbumName by remember { mutableStateOf("") }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add to Album") },
        text = {
            Column(modifier = Modifier.widthIn(min = 260.dp)) {
                if (albums.isEmpty() && !showCreateField) {
                    Text(
                        "No albums yet. Create one to get started.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(vertical = 8.dp)
                    )
                }

                if (albums.isNotEmpty()) {
                    LazyColumn(modifier = Modifier.heightIn(max = 240.dp)) {
                        lazyItems(albums, key = { it.localId }) { album ->
                            Surface(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable { onAlbumSelected(album.localId) },
                                shape = RoundedCornerShape(8.dp)
                            ) {
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .padding(horizontal = 12.dp, vertical = 12.dp),
                                    verticalAlignment = Alignment.CenterVertically
                                ) {
                                    Icon(
                                        painter = painterResource(R.drawable.ic_folder),
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier.size(20.dp)
                                    )
                                    Spacer(Modifier.width(12.dp))
                                    Text(album.name, style = MaterialTheme.typography.bodyLarge)
                                }
                            }
                        }
                    }
                    Spacer(Modifier.height(8.dp))
                }

                if (showCreateField) {
                    OutlinedTextField(
                        value = newAlbumName,
                        onValueChange = { newAlbumName = it },
                        label = { Text("Album name") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth()
                    )
                } else {
                    TextButton(
                        onClick = { showCreateField = true },
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Icon(Icons.Default.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(4.dp))
                        Text("Create New Album")
                    }
                }
            }
        },
        confirmButton = {
            if (showCreateField) {
                Button(
                    onClick = { if (newAlbumName.isNotBlank()) onCreateAlbum(newAlbumName.trim()) },
                    enabled = newAlbumName.isNotBlank()
                ) { Text("Create & Add") }
            }
        },
        dismissButton = {
            OutlinedButton(onClick = onDismiss) { Text("Cancel") }
        }
    )
}

// ── Media Tile ──────────────────────────────────────────────────────────────

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MediaTile(
    photo: PhotoEntity,
    serverBaseUrl: String,
    encryptionMode: String,
    isSelectionMode: Boolean,
    isSelected: Boolean,
    onTap: () -> Unit,
    onLongPress: () -> Unit
) {
    Box(
        modifier = Modifier
            .aspectRatio(1f)
            .clip(MaterialTheme.shapes.small)
            .combinedClickable(
                onClick = onTap,
                onLongClick = onLongPress
            )
    ) {
        val imageModel: Any? = when {
            encryptionMode == "plain" && photo.serverPhotoId != null ->
                "$serverBaseUrl/api/photos/${photo.serverPhotoId}/thumb"
            photo.thumbnailPath != null -> File(photo.thumbnailPath)
            photo.localPath != null -> photo.localPath
            else -> null
        }

        if (imageModel != null) {
            AsyncImage(
                model = ImageRequest.Builder(LocalContext.current)
                    .data(imageModel)
                    .crossfade(true)
                    .size(256)
                    .build(),
                contentDescription = photo.filename,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize()
            )
        } else {
            Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surfaceVariant) {
                Box(contentAlignment = Alignment.Center) {
                    Text(photo.filename.take(8), style = MaterialTheme.typography.labelSmall, textAlign = TextAlign.Center, modifier = Modifier.padding(4.dp))
                }
            }
        }

        // Media type badges
        if (photo.mediaType == "video") {
            Surface(modifier = Modifier.align(Alignment.BottomEnd).padding(4.dp), shape = MaterialTheme.shapes.extraSmall, color = Color.Black.copy(alpha = 0.6f)) {
                Text(
                    text = if (photo.durationSecs != null) { val m = (photo.durationSecs / 60).toInt(); val s = (photo.durationSecs % 60).toInt(); "\u25B6 $m:${s.toString().padStart(2, '0')}" } else "\u25B6",
                    color = Color.White, fontSize = 10.sp, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp)
                )
            }
        } else if (photo.mediaType == "gif") {
            Surface(modifier = Modifier.align(Alignment.BottomEnd).padding(4.dp), shape = MaterialTheme.shapes.extraSmall, color = Color.Black.copy(alpha = 0.6f)) {
                Text("GIF", color = Color.White, fontSize = 10.sp, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp))
            }
        }

        // Sync status indicator (only when not in selection mode)
        if (photo.syncStatus != SyncStatus.SYNCED && !isSelectionMode) {
            Surface(
                modifier = Modifier.align(Alignment.TopEnd).padding(4.dp),
                shape = MaterialTheme.shapes.extraSmall,
                color = when (photo.syncStatus) {
                    SyncStatus.UPLOADING -> Color.Blue.copy(alpha = 0.7f)
                    SyncStatus.FAILED -> Color.Red.copy(alpha = 0.7f)
                    else -> Color.Gray.copy(alpha = 0.7f)
                }
            ) {
                Text(
                    text = when (photo.syncStatus) { SyncStatus.UPLOADING -> "\u2191"; SyncStatus.FAILED -> "!"; else -> "\u2026" },
                    color = Color.White, fontSize = 10.sp, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp)
                )
            }
        }

        // Selection circle (top-right)
        if (isSelectionMode) {
            Box(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(6.dp)
                    .size(24.dp)
                    .clip(CircleShape)
                    .background(if (isSelected) Color(0xFF22C55E) else Color.White.copy(alpha = 0.8f))
                    .border(
                        width = 2.dp,
                        color = if (isSelected) Color(0xFF22C55E) else Color.Gray.copy(alpha = 0.5f),
                        shape = CircleShape
                    ),
                contentAlignment = Alignment.Center
            ) {
                if (isSelected) {
                    Icon(Icons.Default.Check, contentDescription = null, tint = Color.White, modifier = Modifier.size(16.dp))
                }
            }
        }
    }
}
