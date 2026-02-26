package com.simplephotos.ui.screens.gallery

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.*
import androidx.compose.material3.pulltorefresh.PullToRefreshContainer
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.sync.SyncScheduler
import com.simplephotos.ui.components.ActiveTab
import com.simplephotos.ui.components.AppHeader
import com.simplephotos.ui.components.HeaderNavigation
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import com.simplephotos.ui.theme.ThemeState
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import javax.inject.Inject

@HiltViewModel
class GalleryViewModel @Inject constructor(
    private val photoRepository: PhotoRepository,
    private val authRepository: AuthRepository,
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

    /** Server base URL for building thumbnail URLs (plain mode). */
    var serverBaseUrl by mutableStateOf("")
        private set

    /** Current encryption mode ("plain" or "encrypted"). */
    var encryptionMode by mutableStateOf("plain")
        private set

    /** Logged-in username for the header avatar. */
    var username by mutableStateOf("")
        private set

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

    fun logout(onLoggedOut: () -> Unit) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { authRepository.logout() }
                onLoggedOut()
            } catch (_: Exception) {
                onLoggedOut()
            }
        }
    }

    fun deletePhoto(photo: PhotoEntity) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { photoRepository.deletePhoto(photo) }
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    /** Pull new photos from the server. */
    fun syncFromServer() {
        if (isSyncing) return
        viewModelScope.launch {
            isSyncing = true
            error = null
            lastSyncResult = null
            try {
                val url = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val mode = withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() }
                serverBaseUrl = url
                encryptionMode = mode
                val imported = withContext(Dispatchers.IO) { photoRepository.syncFromServer() }
                lastSyncResult = if (imported > 0) "Synced $imported new items" else "Up to date"
            } catch (e: Exception) {
                error = "Sync failed: ${e.message}"
            } finally {
                isSyncing = false
            }
        }
    }

    /** Import photos from local URIs (picked by the user). */
    fun importFromUris(uris: List<Uri>, context: android.content.Context) {
        if (uris.isEmpty()) return
        viewModelScope.launch {
            isImporting = true
            error = null
            var count = 0
            for (uri in uris) {
                try {
                    val resolver = context.contentResolver
                    val mimeType = resolver.getType(uri) ?: "image/jpeg"
                    val mediaType = when {
                        mimeType.startsWith("video/") -> "video"
                        mimeType == "image/gif" -> "gif"
                        else -> "photo"
                    }

                    val data = withContext(Dispatchers.IO) {
                        resolver.openInputStream(uri)?.use { it.readBytes() }
                    } ?: continue

                    // Generate thumbnail
                    val thumbBytes = if (mediaType != "video") {
                        withContext(Dispatchers.IO) {
                            val opts = android.graphics.BitmapFactory.Options().apply {
                                inJustDecodeBounds = true
                            }
                            android.graphics.BitmapFactory.decodeByteArray(data, 0, data.size, opts)
                            val scaleFactor = maxOf(opts.outWidth, opts.outHeight) / 256
                            val opts2 = android.graphics.BitmapFactory.Options().apply {
                                inSampleSize = maxOf(1, scaleFactor)
                            }
                            val bitmap = android.graphics.BitmapFactory.decodeByteArray(data, 0, data.size, opts2)
                            val stream = java.io.ByteArrayOutputStream()
                            bitmap?.compress(android.graphics.Bitmap.CompressFormat.JPEG, 80, stream)
                            bitmap?.recycle()
                            stream.toByteArray()
                        }
                    } else {
                        ByteArray(0)
                    }

                    val localId = java.util.UUID.randomUUID().toString()
                    val filename = uri.lastPathSegment ?: "import_$localId"

                    val photo = PhotoEntity(
                        localId = localId,
                        filename = filename,
                        takenAt = System.currentTimeMillis(),
                        mimeType = mimeType,
                        mediaType = mediaType,
                        width = 0,
                        height = 0,
                        localPath = uri.toString(),
                        syncStatus = SyncStatus.PENDING
                    )

                    photoRepository.insertPhoto(photo)
                    if (thumbBytes.isNotEmpty()) {
                        val thumbPath = photoRepository.saveThumbnailToDisk(localId, thumbBytes)
                        photoRepository.insertPhoto(photo.copy(thumbnailPath = thumbPath))
                    }

                    // Upload based on mode
                    try {
                        withContext(Dispatchers.IO) {
                            if (encryptionMode == "plain") {
                                photoRepository.uploadPhotoPlain(photo, data)
                            } else {
                                photoRepository.uploadPhoto(photo, data, if (thumbBytes.isEmpty()) data else thumbBytes)
                            }
                        }
                    } catch (_: Exception) {}

                    count++
                } catch (e: Exception) {
                    error = "Import error: ${e.message}"
                }
            }
            lastSyncResult = "Imported $count photo${if (count != 1) "s" else ""}"
            isImporting = false
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GalleryScreen(
    onPhotoClick: (String) -> Unit,
    onAlbumsClick: () -> Unit,
    onTrashClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onLogout: () -> Unit,
    viewModel: GalleryViewModel = hiltViewModel()
) {
    val photos by viewModel.photos.collectAsState(initial = emptyList())
    val context = LocalContext.current

    // Photo/video picker launcher
    val pickMediaLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenMultipleDocuments()
    ) { uris: List<Uri> ->
        viewModel.importFromUris(uris, context)
    }

    // Schedule background sync — do not require granular media permissions
    // (we use the document picker for imports, not MediaStore scanning)
    LaunchedEffect(Unit) {
        try { SyncScheduler.schedule(context) } catch (_: Exception) {}
    }

    // Auto-sync on first load: pull from server, then push any pending local photos
    LaunchedEffect(Unit) {
        viewModel.syncFromServer()
        try { SyncScheduler.triggerNow(context) } catch (_: Exception) {}
    }

    Scaffold(
        topBar = {
            AppHeader(
                activeTab = ActiveTab.GALLERY,
                username = viewModel.username,
                navigation = HeaderNavigation(
                    onGalleryClick = { /* already on gallery */ },
                    onAlbumsClick = onAlbumsClick,
                    onTrashClick = onTrashClick,
                    onSettingsClick = onSettingsClick,
                    onLogout = { viewModel.logout(onLogout) },
                    onThemeToggle = { ThemeState.toggle(viewModel.dataStore) }
                ),
                isSyncing = viewModel.isSyncing,
                syncLabel = if (viewModel.isSyncing) "Syncing" else null
            )
        },
        floatingActionButton = {
            FloatingActionButton(
                onClick = { pickMediaLauncher.launch(arrayOf("image/*", "video/*")) },
                containerColor = MaterialTheme.colorScheme.primary
            ) {
                if (viewModel.isImporting) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(24.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary
                    )
                } else {
                    Icon(
                        Icons.Default.Add,
                        contentDescription = "Add photos",
                        tint = MaterialTheme.colorScheme.onPrimary
                    )
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
                Text(
                    msg,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.bodySmall
                )
            }
            viewModel.error?.let { err ->
                Text(
                    err,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.bodySmall
                )
            }

            if (photos.isEmpty() && !viewModel.isSyncing) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text("No photos yet", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Spacer(Modifier.height(8.dp))
                        Text("Tap + to add photos or grant permissions for auto-backup", textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(horizontal = 32.dp))
                    }
                }
            } else if (photos.isEmpty() && viewModel.isSyncing) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        CircularProgressIndicator()
                        Spacer(Modifier.height(16.dp))
                        Text("Syncing from server...")
                    }
                }
            } else {
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(100.dp),
                    contentPadding = PaddingValues(2.dp),
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                    verticalArrangement = Arrangement.spacedBy(2.dp)
                ) {
                    items(photos, key = { it.localId }) { photo ->
                        MediaTile(
                            photo = photo,
                            serverBaseUrl = viewModel.serverBaseUrl,
                            encryptionMode = viewModel.encryptionMode,
                            onClick = { onPhotoClick(photo.localId) }
                        )
                    }
                }
            }
        }
        PullToRefreshContainer(
            state = pullToRefreshState,
            modifier = Modifier.align(Alignment.TopCenter)
        )
        } // end pull-to-refresh Box
    }
}

@Composable
private fun MediaTile(
    photo: PhotoEntity,
    serverBaseUrl: String,
    encryptionMode: String,
    onClick: () -> Unit
) {
    Box(
        modifier = Modifier
            .aspectRatio(1f)
            .clip(MaterialTheme.shapes.small)
            .clickable(onClick = onClick)
    ) {
        val imageModel: Any? = when {
            // Plain mode: use authenticated server thumbnail URL
            encryptionMode == "plain" && photo.serverPhotoId != null ->
                "$serverBaseUrl/api/photos/${photo.serverPhotoId}/thumb"
            // Cached thumbnail (from encrypted sync or upload)
            photo.thumbnailPath != null -> File(photo.thumbnailPath)
            // Local file on device
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

        // Sync status indicator
        if (photo.syncStatus != SyncStatus.SYNCED) {
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
    }
}
