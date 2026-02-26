package com.simplephotos.ui.screens.trash

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Warning
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
import androidx.compose.ui.window.Dialog
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
import com.simplephotos.ui.theme.ThemeState
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

// ── Helpers ─────────────────────────────────────────────────────────────────

private fun formatBytes(bytes: Long): String {
    if (bytes == 0L) return "0 B"
    val k = 1024.0
    val sizes = arrayOf("B", "KB", "MB", "GB")
    val i = (Math.log(bytes.toDouble()) / Math.log(k)).toInt().coerceAtMost(sizes.lastIndex)
    return "%.1f %s".format(bytes / Math.pow(k, i.toDouble()), sizes[i])
}

private fun timeUntil(isoDate: String): String {
    return try {
        val expires = java.time.Instant.parse(isoDate)
        val now = java.time.Instant.now()
        val diff = java.time.Duration.between(now, expires)
        if (diff.isNegative) return "Expiring soon"
        val days = diff.toDays()
        val hours = diff.toHours() % 24
        if (days > 0) "${days}d ${hours}h remaining" else "${hours}h remaining"
    } catch (_: Exception) {
        ""
    }
}

private fun timeSince(isoDate: String): String {
    return try {
        val deleted = java.time.Instant.parse(isoDate)
        val now = java.time.Instant.now()
        val diff = java.time.Duration.between(deleted, now)
        val days = diff.toDays()
        val hours = diff.toHours() % 24
        if (days > 0) "${days}d ago" else if (hours > 0) "${hours}h ago" else "Just now"
    } catch (_: Exception) {
        ""
    }
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

    fun toggleSelect(id: String) {
        selectedIds = if (id in selectedIds) selectedIds - id else selectedIds + id
    }

    fun selectAll() {
        selectedIds = if (selectedIds.size == items.size) emptySet()
        else items.map { it.id }.toSet()
    }

    fun restore(id: String) {
        viewModelScope.launch {
            actionLoading = id
            try {
                withContext(Dispatchers.IO) { api.restoreFromTrash(id) }
                items = items.filter { it.id != id }
                selectedIds = selectedIds - id
            } catch (e: Exception) {
                error = e.message ?: "Failed to restore"
            } finally {
                actionLoading = null
            }
        }
    }

    fun permanentDelete(id: String) {
        viewModelScope.launch {
            actionLoading = id
            try {
                withContext(Dispatchers.IO) { api.permanentDeleteTrash(id) }
                items = items.filter { it.id != id }
                selectedIds = selectedIds - id
            } catch (e: Exception) {
                error = e.message ?: "Failed to delete"
            } finally {
                actionLoading = null
            }
        }
    }

    fun emptyTrash() {
        viewModelScope.launch {
            actionLoading = "empty"
            try {
                withContext(Dispatchers.IO) { api.emptyTrash() }
                items = emptyList()
                selectedIds = emptySet()
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
                selectedIds = emptySet()
            } catch (e: Exception) {
                error = e.message ?: "Failed to restore selected"
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
                selectedIds = emptySet()
            } catch (e: Exception) {
                error = e.message ?: "Failed to delete selected"
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
                    onTrashClick = { /* already on trash */ },
                    onSettingsClick = onSettingsClick,
                    onLogout = { viewModel.logout(onLogout) },
                    onThemeToggle = { ThemeState.toggle(viewModel.dataStore) }
                )
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
        ) {
            // ── Stats Bar ───────────────────────────────────────────
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(
                            Icons.Default.Delete,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.size(28.dp)
                        )
                        Spacer(Modifier.width(12.dp))
                        Text(
                            "Trash",
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.Bold
                        )
                    }
                    Text(
                        if (viewModel.items.isEmpty()) "No items in trash"
                        else "${viewModel.items.size} item${if (viewModel.items.size != 1) "s" else ""} · ${formatBytes(totalSize)} · Items are permanently deleted after 30 days",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 2.dp)
                    )
                }

                if (viewModel.items.isNotEmpty()) {
                    Button(
                        onClick = { showEmptyConfirm = true },
                        enabled = viewModel.actionLoading == null,
                        colors = ButtonDefaults.buttonColors(
                            containerColor = Color(0xFFDC2626),
                            contentColor = Color.White
                        ),
                        shape = RoundedCornerShape(8.dp),
                        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
                    ) {
                        Icon(
                            Icons.Default.Delete,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp)
                        )
                        Spacer(Modifier.width(6.dp))
                        Text("Empty Trash", fontSize = 13.sp, fontWeight = FontWeight.Medium)
                    }
                }
            }

            // ── Bulk action bar (shown when items are selected) ─────
            if (viewModel.selectedIds.isNotEmpty()) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    Surface(
                        shape = RoundedCornerShape(8.dp),
                        color = Color(0xFF065F46).copy(alpha = 0.3f),
                        modifier = Modifier.clickable(
                            enabled = viewModel.actionLoading == null,
                            onClick = { viewModel.restoreSelected() }
                        )
                    ) {
                        Text(
                            "Restore (${viewModel.selectedIds.size})",
                            color = Color(0xFF34D399),
                            fontSize = 12.sp,
                            fontWeight = FontWeight.Medium,
                            modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp)
                        )
                    }
                    Surface(
                        shape = RoundedCornerShape(8.dp),
                        color = Color(0xFF991B1B).copy(alpha = 0.3f),
                        modifier = Modifier.clickable(
                            enabled = viewModel.actionLoading == null,
                            onClick = { viewModel.deleteSelected() }
                        )
                    ) {
                        Text(
                            "Delete (${viewModel.selectedIds.size})",
                            color = Color(0xFFF87171),
                            fontSize = 12.sp,
                            fontWeight = FontWeight.Medium,
                            modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp)
                        )
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
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    CircularProgressIndicator()
                }
            }
            // ── Empty State ─────────────────────────────────────────
            else if (viewModel.items.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Icon(
                            Icons.Default.Delete,
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
                // Select all link
                Row(
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        if (viewModel.selectedIds.size == viewModel.items.size) "Deselect all"
                        else "Select all",
                        color = MaterialTheme.colorScheme.primary,
                        fontSize = 13.sp,
                        modifier = Modifier.clickable { viewModel.selectAll() }
                    )
                    if (viewModel.selectedIds.isNotEmpty()) {
                        Spacer(Modifier.width(12.dp))
                        Text(
                            "${viewModel.selectedIds.size} selected",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            fontSize = 13.sp
                        )
                    }
                }

                LazyVerticalGrid(
                    columns = GridCells.Adaptive(110.dp),
                    contentPadding = PaddingValues(4.dp),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp)
                ) {
                    items(viewModel.items, key = { it.id }) { item ->
                        TrashTile(
                            item = item,
                            serverBaseUrl = viewModel.serverBaseUrl,
                            isSelected = item.id in viewModel.selectedIds,
                            isActionLoading = viewModel.actionLoading != null,
                            onClick = { viewModel.toggleSelect(item.id) },
                            onRestore = { viewModel.restore(item.id) },
                            onDelete = { viewModel.permanentDelete(item.id) }
                        )
                    }
                }
            }
        }
    }

    // ── Empty Trash Confirmation Dialog ──────────────────────────────────
    if (showEmptyConfirm) {
        Dialog(onDismissRequest = { showEmptyConfirm = false }) {
            Surface(
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surface,
                shadowElevation = 8.dp
            ) {
                Column(modifier = Modifier.padding(24.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Surface(
                            shape = CircleShape,
                            color = MaterialTheme.colorScheme.errorContainer,
                            modifier = Modifier.size(40.dp)
                        ) {
                            Box(contentAlignment = Alignment.Center, modifier = Modifier.fillMaxSize()) {
                                Icon(
                                    Icons.Default.Warning,
                                    contentDescription = null,
                                    tint = MaterialTheme.colorScheme.error,
                                    modifier = Modifier.size(20.dp)
                                )
                            }
                        }
                        Spacer(Modifier.width(12.dp))
                        Column {
                            Text(
                                "Empty Trash?",
                                style = MaterialTheme.typography.titleMedium,
                                fontWeight = FontWeight.SemiBold
                            )
                            Text(
                                "This will permanently delete ${viewModel.items.size} item${if (viewModel.items.size != 1) "s" else ""} (${formatBytes(totalSize)}). This action cannot be undone.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(top = 2.dp)
                            )
                        }
                    }
                    Spacer(Modifier.height(24.dp))
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        OutlinedButton(
                            onClick = { showEmptyConfirm = false },
                            modifier = Modifier.weight(1f)
                        ) {
                            Text("Cancel")
                        }
                        Button(
                            onClick = {
                                viewModel.emptyTrash()
                                showEmptyConfirm = false
                            },
                            enabled = viewModel.actionLoading != "empty",
                            colors = ButtonDefaults.buttonColors(
                                containerColor = Color(0xFFDC2626),
                                contentColor = Color.White
                            ),
                            modifier = Modifier.weight(1f)
                        ) {
                            Text(
                                if (viewModel.actionLoading == "empty") "Deleting…" else "Delete All"
                            )
                        }
                    }
                }
            }
        }
    }
}

// ── Trash Tile ──────────────────────────────────────────────────────────────

@Composable
private fun TrashTile(
    item: TrashItemDto,
    serverBaseUrl: String,
    isSelected: Boolean,
    isActionLoading: Boolean,
    onClick: () -> Unit,
    onRestore: () -> Unit,
    onDelete: () -> Unit,
) {
    val thumbUrl = "$serverBaseUrl/api/trash/${item.id}/thumb"

    Box(
        modifier = Modifier
            .aspectRatio(1f)
            .clip(RoundedCornerShape(8.dp))
            .border(
                width = if (isSelected) 2.dp else 0.dp,
                color = if (isSelected) MaterialTheme.colorScheme.primary else Color.Transparent,
                shape = RoundedCornerShape(8.dp)
            )
            .clickable(onClick = onClick)
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

        // Selection checkbox
        Box(
            modifier = Modifier
                .align(Alignment.TopStart)
                .padding(6.dp)
                .size(22.dp)
                .clip(CircleShape)
                .background(
                    if (isSelected) MaterialTheme.colorScheme.primary
                    else Color.White.copy(alpha = 0.7f)
                )
                .border(
                    width = 1.5.dp,
                    color = if (isSelected) MaterialTheme.colorScheme.primary
                    else Color.Gray.copy(alpha = 0.5f),
                    shape = CircleShape
                ),
            contentAlignment = Alignment.Center
        ) {
            if (isSelected) {
                Icon(
                    Icons.Default.Check,
                    contentDescription = null,
                    tint = Color.White,
                    modifier = Modifier.size(14.dp)
                )
            }
        }

        // Media type badge
        if (item.mediaType != "photo") {
            Surface(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(6.dp),
                shape = RoundedCornerShape(4.dp),
                color = Color.Black.copy(alpha = 0.6f)
            ) {
                Text(
                    if (item.mediaType == "video") "Video" else "GIF",
                    color = Color.White,
                    fontSize = 10.sp,
                    fontWeight = FontWeight.Medium,
                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp)
                )
            }
        }

        // Bottom action row
        Row(
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .background(Color.Black.copy(alpha = 0.5f))
                .padding(4.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            Surface(
                modifier = Modifier
                    .weight(1f)
                    .clickable(enabled = !isActionLoading, onClick = onRestore),
                shape = RoundedCornerShape(4.dp),
                color = Color(0xFF059669).copy(alpha = 0.9f)
            ) {
                Text(
                    "Restore",
                    color = Color.White,
                    fontSize = 11.sp,
                    fontWeight = FontWeight.Medium,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(vertical = 4.dp)
                )
            }
            Surface(
                modifier = Modifier
                    .weight(1f)
                    .clickable(enabled = !isActionLoading, onClick = onDelete),
                shape = RoundedCornerShape(4.dp),
                color = Color(0xFFDC2626).copy(alpha = 0.9f)
            ) {
                Text(
                    "Delete",
                    color = Color.White,
                    fontSize = 11.sp,
                    fontWeight = FontWeight.Medium,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(vertical = 4.dp)
                )
            }
        }
    }
}
