package com.simplephotos.ui.screens.album

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
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
import javax.inject.Inject

/**
 * ViewModel for the album detail screen.
 * Loads album metadata and the photos belonging to it.
 * Supports adding/removing photos and deleting the album.
 */
@HiltViewModel
class AlbumDetailViewModel @Inject constructor(
    savedStateHandle: SavedStateHandle,
    private val albumRepository: AlbumRepository,
    private val photoRepository: PhotoRepository
) : ViewModel() {

    val albumId: String = savedStateHandle["albumId"] ?: ""

    var album by mutableStateOf<AlbumEntity?>(null)
    var photos by mutableStateOf<List<PhotoEntity>>(emptyList())
    var allPhotos by mutableStateOf<List<PhotoEntity>>(emptyList())
    var loading by mutableStateOf(true)
    var error by mutableStateOf<String?>(null)
    var showAddPanel by mutableStateOf(false)
    var selectedToAdd by mutableStateOf<Set<String>>(emptySet())
    var showDeleteConfirm by mutableStateOf(false)

    init {
        loadAlbum()
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

    fun confirmAdd() {
        viewModelScope.launch {
            try {
                selectedToAdd.forEach { photoId ->
                    albumRepository.addPhotoToAlbum(photoId, albumId)
                }
                album?.let { albumRepository.syncAlbum(it) }
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
                album?.let { albumRepository.syncAlbum(it) }
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
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AlbumDetailScreen(
    onBack: () -> Unit,
    viewModel: AlbumDetailViewModel = hiltViewModel()
) {
    val context = LocalContext.current

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(viewModel.album?.name ?: "Album") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    IconButton(onClick = { viewModel.openAddPanel() }) {
                        Icon(Icons.Default.Add, contentDescription = "Add photos")
                    }
                    IconButton(onClick = { viewModel.showDeleteConfirm = true }) {
                        Icon(Icons.Default.Delete, contentDescription = "Delete album")
                    }
                }
            )
        }
    ) { padding ->
        Box(modifier = Modifier.padding(padding).fillMaxSize()) {
            when {
                viewModel.loading -> {
                    CircularProgressIndicator(modifier = Modifier.align(Alignment.Center))
                }
                viewModel.photos.isEmpty() -> {
                    Text(
                        "No photos in this album.\nTap + to add some.",
                        modifier = Modifier.align(Alignment.Center),
                        textAlign = TextAlign.Center,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                else -> {
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(120.dp),
                        contentPadding = PaddingValues(8.dp),
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp)
                    ) {
                        items(viewModel.photos, key = { it.localId }) { photo ->
                            AlbumPhotoTile(
                                photo = photo,
                                onRemove = { viewModel.removePhoto(photo.localId) }
                            )
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

    // Add photos panel (bottom sheet style dialog)
    if (viewModel.showAddPanel) {
        AlertDialog(
            onDismissRequest = { viewModel.showAddPanel = false },
            title = { Text("Add Photos") },
            text = {
                if (viewModel.allPhotos.isEmpty()) {
                    Text("All photos are already in this album.")
                } else {
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(100.dp),
                        modifier = Modifier.height(400.dp),
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp)
                    ) {
                        items(viewModel.allPhotos, key = { it.localId }) { photo ->
                            val selected = photo.localId in viewModel.selectedToAdd
                            Box(
                                modifier = Modifier
                                    .aspectRatio(1f)
                                    .clip(MaterialTheme.shapes.small)
                                    .clickable { viewModel.toggleSelection(photo.localId) }
                            ) {
                                // Thumbnail: use local path if available
                                photo.localPath?.let { path ->
                                    AsyncImage(
                                        model = ImageRequest.Builder(context)
                                            .data(path)
                                            .crossfade(true)
                                            .size(200)
                                            .build(),
                                        contentDescription = photo.filename,
                                        contentScale = ContentScale.Crop,
                                        modifier = Modifier.fillMaxSize()
                                    )
                                } ?: Surface(
                                    modifier = Modifier.fillMaxSize(),
                                    color = MaterialTheme.colorScheme.surfaceVariant
                                ) {
                                    Box(contentAlignment = Alignment.Center) {
                                        Text(photo.filename.take(3), style = MaterialTheme.typography.labelSmall)
                                    }
                                }

                                if (selected) {
                                    Box(
                                        modifier = Modifier
                                            .fillMaxSize()
                                            .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.4f)),
                                        contentAlignment = Alignment.Center
                                    ) {
                                        Icon(
                                            Icons.Default.Check,
                                            contentDescription = "Selected",
                                            tint = MaterialTheme.colorScheme.onPrimary
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(
                    onClick = { viewModel.confirmAdd() },
                    enabled = viewModel.selectedToAdd.isNotEmpty()
                ) {
                    Text("Add ${viewModel.selectedToAdd.size}")
                }
            },
            dismissButton = {
                TextButton(onClick = { viewModel.showAddPanel = false }) {
                    Text("Cancel")
                }
            }
        )
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

@Composable
private fun AlbumPhotoTile(
    photo: PhotoEntity,
    onRemove: () -> Unit
) {
    val context = LocalContext.current

    Card(modifier = Modifier.fillMaxWidth()) {
        Box {
            photo.localPath?.let { path ->
                AsyncImage(
                    model = ImageRequest.Builder(context)
                        .data(path)
                        .crossfade(true)
                        .size(240)
                        .build(),
                    contentDescription = photo.filename,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier
                        .fillMaxWidth()
                        .aspectRatio(1f)
                )
            } ?: Surface(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(1f),
                color = MaterialTheme.colorScheme.surfaceVariant
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(photo.filename.take(4), style = MaterialTheme.typography.labelSmall)
                }
            }

            // Media type badge
            if (photo.mediaType == "video") {
                val durationStr = photo.durationSecs?.let { secs ->
                    val m = (secs / 60).toInt()
                    val s = (secs % 60).toInt()
                    "%d:%02d".format(m, s)
                } ?: "▶"
                Surface(
                    modifier = Modifier.align(Alignment.BottomStart).padding(4.dp),
                    color = MaterialTheme.colorScheme.surface.copy(alpha = 0.8f),
                    shape = MaterialTheme.shapes.extraSmall
                ) {
                    Text(durationStr, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp), style = MaterialTheme.typography.labelSmall)
                }
            } else if (photo.mediaType == "gif") {
                Surface(
                    modifier = Modifier.align(Alignment.BottomStart).padding(4.dp),
                    color = MaterialTheme.colorScheme.surface.copy(alpha = 0.8f),
                    shape = MaterialTheme.shapes.extraSmall
                ) {
                    Text("GIF", modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp), style = MaterialTheme.typography.labelSmall)
                }
            }

            // Remove button
            IconButton(
                onClick = onRemove,
                modifier = Modifier.align(Alignment.TopEnd).size(32.dp)
            ) {
                Icon(
                    Icons.Default.Close,
                    contentDescription = "Remove from album",
                    modifier = Modifier.size(18.dp),
                    tint = MaterialTheme.colorScheme.error
                )
            }
        }
    }
}
