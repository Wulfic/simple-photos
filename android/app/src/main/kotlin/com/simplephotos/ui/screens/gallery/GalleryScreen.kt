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
import androidx.compose.material.icons.filled.PhotoAlbum
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
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
import com.google.accompanist.permissions.ExperimentalPermissionsApi
import com.google.accompanist.permissions.isGranted
import com.google.accompanist.permissions.rememberPermissionState
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.sync.SyncScheduler
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.launch
import java.io.File
import javax.inject.Inject

@HiltViewModel
class GalleryViewModel @Inject constructor(
    private val photoRepository: PhotoRepository
) : ViewModel() {
    val photos = photoRepository.getAllPhotos()
    var error by mutableStateOf<String?>(null)
    var isSyncing by mutableStateOf(false)
        private set
    var isImporting by mutableStateOf(false)
        private set
    var lastSyncResult by mutableStateOf<String?>(null)
        private set

    fun deletePhoto(photo: PhotoEntity) {
        viewModelScope.launch {
            try {
                photoRepository.deletePhoto(photo)
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
                val imported = photoRepository.syncFromServer()
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

                    // Read file bytes
                    val data = resolver.openInputStream(uri)?.use { it.readBytes() } ?: continue

                    // Generate a simple thumbnail (reuse bytes for images, placeholder for video)
                    val thumbBytes = if (mediaType != "video") {
                        // Downscale to thumbnail using BitmapFactory
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
                    } else {
                        // For video: use a 1x1 placeholder thumbnail
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

                    // Upload in background
                    try {
                        photoRepository.uploadPhoto(photo, data, if (thumbBytes.isEmpty()) data else thumbBytes)
                    } catch (_: Exception) {
                        // Will show as FAILED in UI, can retry later
                    }

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

@OptIn(ExperimentalPermissionsApi::class, ExperimentalMaterial3Api::class)
@Composable
fun GalleryScreen(
    onPhotoClick: (String) -> Unit,
    onAlbumsClick: () -> Unit,
    onSettingsClick: () -> Unit,
    viewModel: GalleryViewModel = hiltViewModel()
) {
    val photos by viewModel.photos.collectAsState(initial = emptyList())
    val context = LocalContext.current

    // Request media permissions for background sync
    val imagePermission = rememberPermissionState(android.Manifest.permission.READ_MEDIA_IMAGES)
    val videoPermission = rememberPermissionState(android.Manifest.permission.READ_MEDIA_VIDEO)

    // Photo/video picker launcher
    val pickMediaLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenMultipleDocuments()
    ) { uris: List<Uri> ->
        viewModel.importFromUris(uris, context)
    }

    LaunchedEffect(imagePermission.status.isGranted, videoPermission.status.isGranted) {
        if (imagePermission.status.isGranted && videoPermission.status.isGranted) {
            SyncScheduler.schedule(context)
        }
    }

    // Auto-sync on first load
    LaunchedEffect(Unit) {
        viewModel.syncFromServer()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Gallery") },
                actions = {
                    // Sync from server button
                    IconButton(
                        onClick = { viewModel.syncFromServer() },
                        enabled = !viewModel.isSyncing
                    ) {
                        if (viewModel.isSyncing) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(20.dp),
                                strokeWidth = 2.dp
                            )
                        } else {
                            Icon(Icons.Default.Refresh, contentDescription = "Sync from server")
                        }
                    }
                    IconButton(onClick = onAlbumsClick) {
                        Icon(Icons.Default.PhotoAlbum, contentDescription = "Albums")
                    }
                    IconButton(onClick = onSettingsClick) {
                        Icon(Icons.Default.Settings, contentDescription = "Settings")
                    }
                }
            )
        },
        floatingActionButton = {
            FloatingActionButton(
                onClick = {
                    pickMediaLauncher.launch(arrayOf("image/*", "video/*"))
                },
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
        Column(modifier = Modifier.padding(padding)) {
            // Status messages
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

            // Permission request
            if (!imagePermission.status.isGranted || !videoPermission.status.isGranted) {
                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(16.dp)
                ) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        horizontalAlignment = Alignment.CenterHorizontally
                    ) {
                        Text(
                            "Grant access to photos and videos to enable automatic backup",
                            textAlign = TextAlign.Center
                        )
                        Spacer(Modifier.height(8.dp))
                        Button(onClick = {
                            imagePermission.launchPermissionRequest()
                            videoPermission.launchPermissionRequest()
                        }) {
                            Text("Grant Permission")
                        }
                    }
                }
            }

            if (photos.isEmpty() && !viewModel.isSyncing) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            "No photos yet",
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(Modifier.height(8.dp))
                        Text(
                            "Tap + to add photos or grant permissions for auto-backup",
                            textAlign = TextAlign.Center,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.padding(horizontal = 32.dp)
                        )
                    }
                }
            } else if (photos.isEmpty() && viewModel.isSyncing) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        CircularProgressIndicator()
                        Spacer(Modifier.height(16.dp))
                        Text("Syncing from server...")
                    }
                }
            } else {
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(120.dp),
                    contentPadding = PaddingValues(4.dp),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp)
                ) {
                    items(photos, key = { it.localId }) { photo ->
                        MediaTile(
                            photo = photo,
                            onClick = { onPhotoClick(photo.localId) }
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun MediaTile(photo: PhotoEntity, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .aspectRatio(1f)
            .clip(MaterialTheme.shapes.small)
            .clickable(onClick = onClick)
    ) {
        // Determine the image source: cached thumbnail > local file > placeholder
        val imageModel: Any? = when {
            // Cached thumbnail (from sync or upload)
            photo.thumbnailPath != null -> File(photo.thumbnailPath)
            // Local file on device (from background scan)
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
            // Fallback placeholder for photos without any local image
            Surface(
                modifier = Modifier.fillMaxSize(),
                color = MaterialTheme.colorScheme.surfaceVariant
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(
                        photo.filename.take(8),
                        style = MaterialTheme.typography.labelSmall,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.padding(4.dp)
                    )
                }
            }
        }

        // Media type badges
        if (photo.mediaType == "video") {
            Surface(
                modifier = Modifier
                    .align(Alignment.BottomEnd)
                    .padding(4.dp),
                shape = MaterialTheme.shapes.extraSmall,
                color = Color.Black.copy(alpha = 0.6f)
            ) {
                Text(
                    text = if (photo.durationSecs != null) {
                        val m = (photo.durationSecs / 60).toInt()
                        val s = (photo.durationSecs % 60).toInt()
                        "\u25B6 $m:${s.toString().padStart(2, '0')}"
                    } else "\u25B6",
                    color = Color.White,
                    fontSize = 10.sp,
                    modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp)
                )
            }
        } else if (photo.mediaType == "gif") {
            Surface(
                modifier = Modifier
                    .align(Alignment.BottomEnd)
                    .padding(4.dp),
                shape = MaterialTheme.shapes.extraSmall,
                color = Color.Black.copy(alpha = 0.6f)
            ) {
                Text(
                    "GIF",
                    color = Color.White,
                    fontSize = 10.sp,
                    modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp)
                )
            }
        }

        // Sync status indicator
        if (photo.syncStatus != SyncStatus.SYNCED) {
            Surface(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(4.dp),
                shape = MaterialTheme.shapes.extraSmall,
                color = when (photo.syncStatus) {
                    SyncStatus.UPLOADING -> Color.Blue.copy(alpha = 0.7f)
                    SyncStatus.FAILED -> Color.Red.copy(alpha = 0.7f)
                    else -> Color.Gray.copy(alpha = 0.7f)
                }
            ) {
                Text(
                    text = when (photo.syncStatus) {
                        SyncStatus.UPLOADING -> "\u2191"
                        SyncStatus.FAILED -> "!"
                        else -> "\u2026"
                    },
                    color = Color.White,
                    fontSize = 10.sp,
                    modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp)
                )
            }
        }
    }
}
