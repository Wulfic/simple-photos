package com.simplephotos.ui.screens.album

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.repository.AlbumRepository
import com.simplephotos.data.repository.PhotoRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import java.io.File
import javax.inject.Inject
import com.simplephotos.R
import androidx.compose.ui.res.painterResource

@HiltViewModel
class AlbumDetailViewModel @Inject constructor(
    savedStateHandle: SavedStateHandle,
    private val albumRepository: AlbumRepository,
    private val photoRepository: PhotoRepository
) : ViewModel() {

    val albumId: String = savedStateHandle["albumId"] ?: ""

    /** Whether this is a virtual smart album (favorites, photos, gifs, videos) */
    val isSmartAlbum: Boolean = albumId.startsWith("smart-")

    /** Human-readable label for smart albums */
    val smartAlbumLabel: String = when (albumId) {
        "smart-favorites" -> "Favorites"
        "smart-photos" -> "Photos"
        "smart-gifs" -> "GIFs"
        "smart-videos" -> "Videos"
        "smart-audio" -> "Audio"
        else -> "Album"
    }

    var album by mutableStateOf<AlbumEntity?>(null)
    var photos by mutableStateOf<List<PhotoEntity>>(emptyList())
    var allPhotos by mutableStateOf<List<PhotoEntity>>(emptyList())
    var loading by mutableStateOf(true)
    var error by mutableStateOf<String?>(null)
    var showAddPanel by mutableStateOf(false)
    var selectedToAdd by mutableStateOf<Set<String>>(emptySet())
    var showDeleteConfirm by mutableStateOf(false)

    var serverBaseUrl by mutableStateOf("")
        private set
    var encryptionMode by mutableStateOf("plain")
        private set

    // ── Multi-select state ────────────────────────────────────────
    var selectedIds by mutableStateOf(emptySet<String>())
        private set
    var isSelectionMode by mutableStateOf(false)
        private set

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = photoRepository.getServerBaseUrl()
                encryptionMode = photoRepository.getEncryptionMode()
            } catch (_: Exception) {}
        }
        if (isSmartAlbum) loadSmartAlbum() else loadAlbum()
    }

    /** Load filtered photos for smart albums */
    private fun loadSmartAlbum() {
        viewModelScope.launch {
            loading = true
            try {
                val all = photoRepository.getAllPhotos().first()
                photos = when (albumId) {
                    "smart-favorites" -> all.filter { it.isFavorite }
                    "smart-photos" -> all.filter { it.mediaType == "photo" || it.mediaType == "gif" }
                    "smart-gifs" -> all.filter { it.mediaType == "gif" }
                    "smart-videos" -> all.filter { it.mediaType == "video" }
                    "smart-audio" -> all.filter { it.mediaType == "audio" }
                    else -> all
                }
            } catch (e: Exception) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }

    fun loadAlbum() {
        viewModelScope.launch {
            loading = true
            try {
                album = albumRepository.getAlbum(albumId)
                val photoIds = albumRepository.getPhotoIdsForAlbum(albumId)
                photos = photoIds.mapNotNull { id ->
                    photoRepository.getPhoto(id)
                }
            } catch (e: Exception) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }

    fun openAddPanel() {
        viewModelScope.launch {
            val existingIds = photos.map { it.localId }.toSet()
            allPhotos = photoRepository.getAllPhotos().first().filter { it.localId !in existingIds }
            selectedToAdd = emptySet()
            showAddPanel = true
        }
    }

    fun toggleSelection(photoId: String) {
        selectedToAdd = if (photoId in selectedToAdd) {
            selectedToAdd - photoId
        } else {
            selectedToAdd + photoId
        }
    }

    fun selectAllAvailable() {
        selectedToAdd = allPhotos.map { it.localId }.toSet()
    }

    fun confirmAdd() {
        viewModelScope.launch {
            try {
                selectedToAdd.forEach { photoId ->
                    albumRepository.addPhotoToAlbum(photoId, albumId)
                }
                // Only sync album manifest in encrypted mode
                try {
                    album?.let { albumRepository.syncAlbum(it) }
                } catch (_: Exception) {
                    // Sync may fail in plain mode — album data is still stored locally
                }
                showAddPanel = false
                loadAlbum()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun removePhoto(photoId: String) {
        viewModelScope.launch {
            try {
                albumRepository.removePhotoFromAlbum(photoId, albumId)
                try {
                    album?.let { albumRepository.syncAlbum(it) }
                } catch (_: Exception) {}
                loadAlbum()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    fun deleteAlbum(onDeleted: () -> Unit) {
        viewModelScope.launch {
            try {
                album?.let { albumRepository.deleteAlbum(it) }
                onDeleted()
            } catch (e: Exception) {
                error = e.message
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

    fun selectAll() {
        isSelectionMode = true
        selectedIds = photos.map { it.localId }.toSet()
    }

    fun clearSelection() {
        selectedIds = emptySet()
        isSelectionMode = false
    }

    fun removeSelectedFromAlbum() {
        viewModelScope.launch {
            try {
                for (id in selectedIds) {
                    albumRepository.removePhotoFromAlbum(id, albumId)
                }
                try {
                    album?.let { albumRepository.syncAlbum(it) }
                } catch (_: Exception) {}
                clearSelection()
                loadAlbum()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }
}

// ── Screen ──────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun AlbumDetailScreen(
    onBack: () -> Unit,
    onPhotoClick: (String) -> Unit = {},
    viewModel: AlbumDetailViewModel = hiltViewModel()
) {
    val context = LocalContext.current

    Scaffold(
        topBar = {
            if (viewModel.isSelectionMode) {
                // ── Selection mode top bar ─────────────────────────
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
                                onClick = { viewModel.selectAll() },
                                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp)
                            ) {
                                Text("Select All", fontSize = 12.sp)
                            }
                        }
                        Button(
                            onClick = { viewModel.removeSelectedFromAlbum() },
                            enabled = viewModel.selectedIds.isNotEmpty(),
                            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp)
                        ) {
                            Icon(Icons.Default.Close, contentDescription = null, modifier = Modifier.size(16.dp))
                            Spacer(Modifier.width(4.dp))
                            Text("Remove", fontSize = 12.sp)
                        }
                    }
                }
            } else {
                TopAppBar(
                    title = {
                        Text(
                            if (viewModel.isSmartAlbum) viewModel.smartAlbumLabel
                            else (viewModel.album?.name ?: "Album")
                        )
                    },
                    navigationIcon = {
                        IconButton(onClick = onBack) {
                            Icon(
                                painter = painterResource(R.drawable.ic_back_arrow),
                                contentDescription = "Back",
                                modifier = Modifier.size(12.dp)
                            )
                        }
                    },
                    actions = {
                        if (!viewModel.isSmartAlbum) {
                            IconButton(onClick = { viewModel.showDeleteConfirm = true }) {
                                Icon(
                                    painter = painterResource(R.drawable.ic_trashcan),
                                    contentDescription = "Delete album",
                                    modifier = Modifier.size(12.dp)
                                )
                            }
                        }
                    }
                )
            }
        }
    ) { padding ->
        Box(modifier = Modifier.padding(padding).fillMaxSize()) {
            when {
                viewModel.loading -> {
                    CircularProgressIndicator(modifier = Modifier.align(Alignment.Center))
                }
                viewModel.showAddPanel && !viewModel.isSmartAlbum -> {
                    // ── Add Photos Panel (full content area) ─────────────
                    Column(modifier = Modifier.fillMaxSize()) {
                        // Header bar at top of page
                        Surface(
                            modifier = Modifier.fillMaxWidth(),
                            color = MaterialTheme.colorScheme.surfaceVariant,
                            shadowElevation = 1.dp
                        ) {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 12.dp, vertical = 8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.SpaceBetween
                            ) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    IconButton(onClick = { viewModel.showAddPanel = false }, modifier = Modifier.size(32.dp)) {
                                        Icon(Icons.Default.Close, contentDescription = "Cancel", modifier = Modifier.size(20.dp))
                                    }
                                    Spacer(Modifier.width(4.dp))
                                    Text(
                                        "${viewModel.selectedToAdd.size} selected",
                                        style = MaterialTheme.typography.titleSmall,
                                        fontWeight = FontWeight.Medium
                                    )
                                    Spacer(Modifier.width(8.dp))
                                    TextButton(
                                        onClick = { viewModel.selectAllAvailable() },
                                        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp)
                                    ) {
                                        Text("Select All", fontSize = 12.sp)
                                    }
                                }
                                Button(
                                    onClick = { viewModel.confirmAdd() },
                                    enabled = viewModel.selectedToAdd.isNotEmpty(),
                                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 6.dp)
                                ) {
                                    Icon(Icons.Default.Add, contentDescription = null, modifier = Modifier.size(16.dp))
                                    Spacer(Modifier.width(4.dp))
                                    Text("Add ${if (viewModel.selectedToAdd.isNotEmpty()) viewModel.selectedToAdd.size else ""}", fontSize = 13.sp)
                                }
                            }
                        }

                        // Photo grid
                        if (viewModel.allPhotos.isEmpty()) {
                            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                                Text(
                                    "All photos are already in this album.",
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                        } else {
                            LazyVerticalGrid(
                                columns = GridCells.Adaptive(100.dp),
                                contentPadding = PaddingValues(2.dp),
                                horizontalArrangement = Arrangement.spacedBy(2.dp),
                                verticalArrangement = Arrangement.spacedBy(2.dp)
                            ) {
                                items(viewModel.allPhotos, key = { it.localId }) { photo ->
                                    val selected = photo.localId in viewModel.selectedToAdd
                                    AddPhotoTile(
                                        photo = photo,
                                        serverBaseUrl = viewModel.serverBaseUrl,
                                        encryptionMode = viewModel.encryptionMode,
                                        isSelected = selected,
                                        onClick = { viewModel.toggleSelection(photo.localId) }
                                    )
                                }
                            }
                        }
                    }
                }
                viewModel.photos.isEmpty() -> {
                    Column(
                        modifier = Modifier.fillMaxSize(),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.Center
                    ) {
                        Icon(
                            painter = painterResource(R.drawable.ic_folder),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f),
                            modifier = Modifier.size(64.dp)
                        )
                        Spacer(Modifier.height(16.dp))
                        Text(
                            if (viewModel.isSmartAlbum) "No ${viewModel.smartAlbumLabel.lowercase()} found."
                            else "No photos in this album.",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center
                        )
                        if (!viewModel.isSmartAlbum) {
                            Spacer(Modifier.height(12.dp))
                            Button(onClick = { viewModel.openAddPanel() }) {
                                Icon(Icons.Default.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                                Spacer(Modifier.width(6.dp))
                                Text("Add Photos")
                            }
                        }
                    }
                }
                else -> {
                    Column(modifier = Modifier.fillMaxSize()) {
                        // Add photos button at top of page (not for smart albums)
                        if (!viewModel.isSmartAlbum) {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 16.dp, vertical = 8.dp),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Text(
                                    "${viewModel.photos.size} photo${if (viewModel.photos.size != 1) "s" else ""}",
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                                OutlinedButton(onClick = { viewModel.openAddPanel() }) {
                                    Icon(Icons.Default.Add, contentDescription = null, modifier = Modifier.size(16.dp))
                                    Spacer(Modifier.width(4.dp))
                                    Text("Add Photos", fontSize = 13.sp)
                                }
                            }
                        } else {
                            // Just show photo count for smart albums
                            Text(
                                "${viewModel.photos.size} item${if (viewModel.photos.size != 1) "s" else ""}",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)
                            )
                        }

                        LazyVerticalGrid(
                            columns = GridCells.Adaptive(100.dp),
                            contentPadding = PaddingValues(2.dp),
                            horizontalArrangement = Arrangement.spacedBy(2.dp),
                            verticalArrangement = Arrangement.spacedBy(2.dp)
                        ) {
                            items(viewModel.photos, key = { it.localId }) { photo ->
                                AlbumPhotoTile(
                                    photo = photo,
                                    serverBaseUrl = viewModel.serverBaseUrl,
                                    encryptionMode = viewModel.encryptionMode,
                                    isSelectionMode = viewModel.isSelectionMode,
                                    isSelected = photo.localId in viewModel.selectedIds,
                                    onTap = {
                                        if (viewModel.isSelectionMode) viewModel.toggleSelect(photo.localId)
                                        else onPhotoClick(photo.localId)
                                    },
                                    onLongPress = { viewModel.enterSelectionMode(photo.localId) }
                                )
                            }
                        }
                    }
                }
            }

            viewModel.error?.let { err ->
                Snackbar(
                    modifier = Modifier.align(Alignment.BottomCenter).padding(16.dp)
                ) {
                    Text(err)
                }
            }
        }
    }

    // Delete album confirmation
    if (viewModel.showDeleteConfirm) {
        AlertDialog(
            onDismissRequest = { viewModel.showDeleteConfirm = false },
            title = { Text("Delete Album") },
            text = { Text("Are you sure you want to delete \"${viewModel.album?.name}\"? Photos will not be deleted.") },
            confirmButton = {
                TextButton(onClick = { viewModel.deleteAlbum(onBack) }) {
                    Text("Delete", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { viewModel.showDeleteConfirm = false }) {
                    Text("Cancel")
                }
            }
        )
    }
}

// ── Add Photo Tile ──────────────────────────────────────────────────────────

@Composable
private fun AddPhotoTile(
    photo: PhotoEntity,
    serverBaseUrl: String,
    encryptionMode: String,
    isSelected: Boolean,
    onClick: () -> Unit
) {
    val context = LocalContext.current

    Box(
        modifier = Modifier
            .aspectRatio(1f)
            .clip(MaterialTheme.shapes.small)
            .clickable(onClick = onClick)
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
                model = ImageRequest.Builder(context)
                    .data(imageModel)
                    .crossfade(true)
                    .size(256)
                    .build(),
                contentDescription = photo.filename,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize()
            )
        } else {
            Surface(
                modifier = Modifier.fillMaxSize(),
                color = MaterialTheme.colorScheme.surfaceVariant
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(photo.filename.take(3), style = MaterialTheme.typography.labelSmall)
                }
            }
        }

        // Selection circle (top-right)
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

// ── Album Photo Tile ────────────────────────────────────────────────────────

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun AlbumPhotoTile(
    photo: PhotoEntity,
    serverBaseUrl: String,
    encryptionMode: String,
    isSelectionMode: Boolean,
    isSelected: Boolean,
    onTap: () -> Unit,
    onLongPress: () -> Unit
) {
    val context = LocalContext.current

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
                model = ImageRequest.Builder(context)
                    .data(imageModel)
                    .crossfade(true)
                    .size(256)
                    .build(),
                contentDescription = photo.filename,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize()
            )
        } else {
            Surface(
                modifier = Modifier.fillMaxSize(),
                color = MaterialTheme.colorScheme.surfaceVariant
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(photo.filename.take(4), style = MaterialTheme.typography.labelSmall)
                }
            }
        }

        // Media type badge
        if (photo.mediaType == "video") {
            val durationStr = photo.durationSecs?.let { secs ->
                val m = (secs / 60).toInt()
                val s = (secs % 60).toInt()
                "%d:%02d".format(m, s)
            } ?: "\u25B6"
            Surface(
                modifier = Modifier.align(Alignment.BottomEnd).padding(4.dp),
                color = Color.Black.copy(alpha = 0.6f),
                shape = MaterialTheme.shapes.extraSmall
            ) {
                Text(durationStr, color = Color.White, fontSize = 10.sp, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp))
            }
        } else if (photo.mediaType == "gif") {
            Surface(
                modifier = Modifier.align(Alignment.BottomEnd).padding(4.dp),
                color = Color.Black.copy(alpha = 0.6f),
                shape = MaterialTheme.shapes.extraSmall
            ) {
                Text("GIF", color = Color.White, fontSize = 10.sp, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp))
            }
        }

        // Selection circle (top-right) — only visible in selection mode
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
