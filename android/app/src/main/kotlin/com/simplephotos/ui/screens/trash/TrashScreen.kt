package com.simplephotos.ui.screens.trash

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
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.TrashItemDto
import com.simplephotos.data.repository.AuthRepository
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.ui.components.ActiveTab
import com.simplephotos.ui.components.AppHeader
import com.simplephotos.ui.components.HeaderNavigation
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject
import com.simplephotos.R
import androidx.compose.ui.res.painterResource

// ── Helpers ─────────────────────────────────────────────────────────────────

private fun formatBytes(bytes: Long): String {
    if (bytes == 0L) return "0 B"
    val k = 1024.0
    val sizes = arrayOf("B", "KB", "MB", "GB")
    val i = (Math.log(bytes.toDouble()) / Math.log(k)).toInt().coerceAtMost(sizes.lastIndex)
    return "%.1f %s".format(bytes / Math.pow(k, i.toDouble()), sizes[i])
}

// ── ViewModel ───────────────────────────────────────────────────────────────

@HiltViewModel
class TrashViewModel @Inject constructor(
    private val api: ApiService,
    private val photoRepository: PhotoRepository,
    private val authRepository: AuthRepository,
    val dataStore: DataStore<Preferences>
) : ViewModel() {
    var items by mutableStateOf<List<TrashItemDto>>(emptyList())
        private set
    var isLoading by mutableStateOf(true)
        private set
    var error by mutableStateOf<String?>(null)
    var actionLoading by mutableStateOf<String?>(null)
        private set
    var selectedIds by mutableStateOf(emptySet<String>())
        private set
    var isSelectionMode by mutableStateOf(false)
        private set
    var serverBaseUrl by mutableStateOf("")
        private set
    var username by mutableStateOf("")
        private set

    init {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                val prefs = dataStore.data.first()
                username = prefs[KEY_USERNAME] ?: ""
            } catch (_: Exception) {}
            loadTrash()
        }
    }

    fun loadTrash() {
        viewModelScope.launch {
            isLoading = true
            error = null
            try {
                val resp = withContext(Dispatchers.IO) { api.listTrash(limit = 500) }
                items = resp.items
            } catch (e: Exception) {
                error = e.message ?: "Failed to load trash"
            } finally {
                isLoading = false
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

    fun clearSelection() {
        selectedIds = emptySet()
        isSelectionMode = false
    }

    fun emptyTrash() {
        viewModelScope.launch {
            actionLoading = "empty"
            try {
                withContext(Dispatchers.IO) { api.emptyTrash() }
                items = emptyList()
                clearSelection()
            } catch (e: Exception) {
                error = e.message ?: "Failed to empty trash"
            } finally {
                actionLoading = null
            }
        }
    }

    fun restoreSelected() {
        viewModelScope.launch {
            actionLoading = "bulk-restore"
            try {
                for (id in selectedIds) {
                    withContext(Dispatchers.IO) { api.restoreFromTrash(id) }
                }
                items = items.filter { it.id !in selectedIds }
                clearSelection()
            } catch (e: Exception) {
                error = e.message ?: "Failed to restore"
                loadTrash()
            } finally {
                actionLoading = null
            }
        }
    }

    fun deleteSelected() {
        viewModelScope.launch {
            actionLoading = "bulk-delete"
            try {
                for (id in selectedIds) {
                    withContext(Dispatchers.IO) { api.permanentDeleteTrash(id) }
                }
                items = items.filter { it.id !in selectedIds }
                clearSelection()
            } catch (e: Exception) {
                error = e.message ?: "Failed to delete"
                loadTrash()
            } finally {
                actionLoading = null
            }
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
}

// ── Screen ──────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TrashScreen(
    onGalleryClick: () -> Unit,
    onAlbumsClick: () -> Unit,
    onSearchClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onLogout: () -> Unit,
    viewModel: TrashViewModel = hiltViewModel()
) {
    var showEmptyConfirm by remember { mutableStateOf(false) }
    val totalSize = viewModel.items.sumOf { it.sizeBytes }

    Scaffold(
        topBar = {
            AppHeader(
                activeTab = ActiveTab.TRASH,
                username = viewModel.username,
                navigation = HeaderNavigation(
                    onGalleryClick = onGalleryClick,
                    onAlbumsClick = onAlbumsClick,
                    onSearchClick = onSearchClick,
                    onTrashClick = { /* already on trash */ },
                    onSettingsClick = onSettingsClick,
                    onLogout = { viewModel.logout(onLogout) }
                )
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
        ) {
            // ── Header / Selection bar ──────────────────────────────
            if (viewModel.isSelectionMode) {
                // Selection action bar
                Surface(
                    modifier = Modifier.fillMaxWidth(),
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    shadowElevation = 2.dp
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            IconButton(onClick = { viewModel.clearSelection() }, modifier = Modifier.size(32.dp)) {
                                Icon(Icons.Default.Close, contentDescription = "Cancel", modifier = Modifier.size(20.dp))
                            }
                            Spacer(Modifier.width(8.dp))
                            Text(
                                "${viewModel.selectedIds.size} selected",
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = FontWeight.Medium
                            )
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            OutlinedButton(
                                onClick = { viewModel.restoreSelected() },
                                enabled = viewModel.actionLoading == null && viewModel.selectedIds.isNotEmpty()
                            ) {
                                Icon(painter = painterResource(R.drawable.ic_reload), contentDescription = null, modifier = Modifier.size(16.dp))
                                Spacer(Modifier.width(4.dp))
                                Text("Restore", fontSize = 13.sp)
                            }
                            Button(
                                onClick = { viewModel.deleteSelected() },
                                enabled = viewModel.actionLoading == null && viewModel.selectedIds.isNotEmpty()
                            ) {
                                Icon(painter = painterResource(R.drawable.ic_trashcan), contentDescription = null, modifier = Modifier.size(16.dp))
                                Spacer(Modifier.width(4.dp))
                                Text("Delete", fontSize = 13.sp)
                            }
                        }
                    }
                }
            } else {
                // Normal header
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            "Trash",
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.Bold
                        )
                        if (viewModel.items.isNotEmpty()) {
                            Text(
                                "${viewModel.items.size} item${if (viewModel.items.size != 1) "s" else ""} · ${formatBytes(totalSize)} · Deleted after 30 days",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(top = 2.dp)
                            )
                        }
                    }

                    if (viewModel.items.isNotEmpty()) {
                        OutlinedButton(
                            onClick = { showEmptyConfirm = true },
                            enabled = viewModel.actionLoading == null
                        ) {
                            Text("Empty Trash", fontSize = 13.sp)
                        }
                    }
                }
            }

            // ── Error ───────────────────────────────────────────────
            viewModel.error?.let { err ->
                Surface(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp),
                    shape = RoundedCornerShape(8.dp),
                    color = MaterialTheme.colorScheme.errorContainer
                ) {
                    Text(
                        err,
                        modifier = Modifier.padding(12.dp),
                        color = MaterialTheme.colorScheme.onErrorContainer,
                        style = MaterialTheme.typography.bodySmall
                    )
                }
            }

            // ── Loading ─────────────────────────────────────────────
            if (viewModel.isLoading) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
            }
            // ── Empty State ─────────────────────────────────────────
            else if (viewModel.items.isEmpty()) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Icon(
                            painter = painterResource(R.drawable.ic_trashcan),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f),
                            modifier = Modifier.size(64.dp)
                        )
                        Spacer(Modifier.height(16.dp))
                        Text(
                            "Trash is empty",
                            style = MaterialTheme.typography.titleLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f)
                        )
                        Text(
                            "Deleted photos will appear here for 30 days",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f),
                            modifier = Modifier.padding(top = 4.dp)
                        )
                    }
                }
            }
            // ── Grid ────────────────────────────────────────────────
            else {
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(100.dp),
                    contentPadding = PaddingValues(2.dp),
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                    verticalArrangement = Arrangement.spacedBy(2.dp)
                ) {
                    items(viewModel.items, key = { it.id }) { item ->
                        TrashTile(
                            item = item,
                            serverBaseUrl = viewModel.serverBaseUrl,
                            isSelectionMode = viewModel.isSelectionMode,
                            isSelected = item.id in viewModel.selectedIds,
                            onLongPress = { viewModel.enterSelectionMode(item.id) },
                            onTap = {
                                if (viewModel.isSelectionMode) {
                                    viewModel.toggleSelect(item.id)
                                }
                            }
                        )
                    }
                }
            }
        }
    }

    // ── Empty Trash Confirmation Dialog ──────────────────────────────────
    if (showEmptyConfirm) {
        AlertDialog(
            onDismissRequest = { showEmptyConfirm = false },
            icon = { Icon(Icons.Default.Warning, contentDescription = null) },
            title = { Text("Empty Trash?") },
            text = {
                Text("This will permanently delete ${viewModel.items.size} item${if (viewModel.items.size != 1) "s" else ""} (${formatBytes(totalSize)}). This cannot be undone.")
            },
            confirmButton = {
                Button(
                    onClick = {
                        viewModel.emptyTrash()
                        showEmptyConfirm = false
                    },
                    enabled = viewModel.actionLoading != "empty"
                ) {
                    Text(if (viewModel.actionLoading == "empty") "Deleting…" else "Delete All")
                }
            },
            dismissButton = {
                OutlinedButton(onClick = { showEmptyConfirm = false }) {
                    Text("Cancel")
                }
            }
        )
    }
}

// ── Trash Tile ──────────────────────────────────────────────────────────────

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun TrashTile(
    item: TrashItemDto,
    serverBaseUrl: String,
    isSelectionMode: Boolean,
    isSelected: Boolean,
    onLongPress: () -> Unit,
    onTap: () -> Unit,
) {
    val thumbUrl = "$serverBaseUrl/api/trash/${item.id}/thumb"

    Box(
        modifier = Modifier
            .aspectRatio(1f)
            .clip(MaterialTheme.shapes.small)
            .combinedClickable(
                onClick = onTap,
                onLongClick = onLongPress
            )
    ) {
        // Thumbnail
        AsyncImage(
            model = ImageRequest.Builder(LocalContext.current)
                .data(thumbUrl)
                .crossfade(true)
                .size(256)
                .build(),
            contentDescription = item.filename,
            contentScale = ContentScale.Crop,
            modifier = Modifier.fillMaxSize()
        )

        // Media type badge
        if (item.mediaType == "video") {
            Surface(
                modifier = Modifier.align(Alignment.BottomEnd).padding(4.dp),
                shape = MaterialTheme.shapes.extraSmall,
                color = Color.Black.copy(alpha = 0.6f)
            ) {
                Text(
                    text = if (item.durationSecs != null) {
                        val m = (item.durationSecs / 60).toInt()
                        val s = (item.durationSecs % 60).toInt()
                        "\u25B6 $m:${s.toString().padStart(2, '0')}"
                    } else "\u25B6",
                    color = Color.White, fontSize = 10.sp,
                    modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp)
                )
            }
        } else if (item.mediaType == "gif") {
            Surface(
                modifier = Modifier.align(Alignment.BottomEnd).padding(4.dp),
                shape = MaterialTheme.shapes.extraSmall,
                color = Color.Black.copy(alpha = 0.6f)
            ) {
                Text("GIF", color = Color.White, fontSize = 10.sp, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp))
            }
        }

        // Selection circle (top-right) — always shown, empty when not selected
        if (isSelectionMode) {
            Box(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(6.dp)
                    .size(24.dp)
                    .clip(CircleShape)
                    .background(
                        if (isSelected) Color(0xFF22C55E) else Color.White.copy(alpha = 0.8f)
                    )
                    .border(
                        width = 2.dp,
                        color = if (isSelected) Color(0xFF22C55E) else Color.Gray.copy(alpha = 0.5f),
                        shape = CircleShape
                    ),
                contentAlignment = Alignment.Center
            ) {
                if (isSelected) {
                    Icon(
                        Icons.Default.Check,
                        contentDescription = null,
                        tint = Color.White,
                        modifier = Modifier.size(16.dp)
                    )
                }
            }
        }
    }
}
