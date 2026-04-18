/**
 * Composable screen for viewing and restoring or permanently deleting trashed photos.
 */
package com.simplephotos.ui.screens.trash

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
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
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.remote.dto.TrashItemDto
import com.simplephotos.ui.components.ActiveTab
import com.simplephotos.ui.components.AppHeader
import com.simplephotos.ui.components.HeaderNavigation
import com.simplephotos.ui.components.JustifiedGrid
import com.simplephotos.ui.theme.ThemeState
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

// ── Screen ──────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TrashScreen(
    onGalleryClick: () -> Unit,
    onAlbumsClick: () -> Unit,
    onSearchClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onSecureGalleryClick: () -> Unit = {},
    onSharedAlbumsClick: () -> Unit = {},
    onDiagnosticsClick: () -> Unit = {},
    onLogout: () -> Unit,
    isAdmin: Boolean = false,
    viewModel: TrashViewModel = hiltViewModel()
) {
    var showEmptyConfirm by remember { mutableStateOf(false) }
    val totalSize = viewModel.items.sumOf { it.sizeBytes }
    val isSystemDark = androidx.compose.foundation.isSystemInDarkTheme()

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
            // ── Header ──────────────────────────────────────────────
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp)
            ) {
                // Title & subtitle
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

                // Action buttons — stacked below the header
                if (viewModel.items.isNotEmpty()) {
                    Spacer(Modifier.height(12.dp))
                    OutlinedButton(
                        onClick = { showEmptyConfirm = true },
                        enabled = viewModel.actionLoading == null
                    ) {
                        Icon(painter = painterResource(R.drawable.ic_trashcan), contentDescription = null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(4.dp))
                        Text("Empty Trash", fontSize = 13.sp)
                    }
                }

                // Selection-mode buttons below empty trash
                if (viewModel.isSelectionMode && viewModel.selectedIds.isNotEmpty()) {
                    Spacer(Modifier.height(8.dp))
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        IconButton(onClick = { viewModel.clearSelection() }, modifier = Modifier.size(32.dp)) {
                            Icon(Icons.Default.Close, contentDescription = "Cancel", modifier = Modifier.size(20.dp))
                        }
                        Text(
                            "${viewModel.selectedIds.size} selected",
                            style = MaterialTheme.typography.titleSmall,
                            fontWeight = FontWeight.Medium
                        )
                        Spacer(Modifier.weight(1f))
                        OutlinedButton(
                            onClick = { viewModel.restoreSelected() },
                            enabled = viewModel.actionLoading == null
                        ) {
                            Icon(painter = painterResource(R.drawable.ic_reload), contentDescription = null, modifier = Modifier.size(16.dp))
                            Spacer(Modifier.width(4.dp))
                            Text("Restore", fontSize = 13.sp)
                        }
                        Button(
                            onClick = { viewModel.deleteSelected() },
                            enabled = viewModel.actionLoading == null
                        ) {
                            Icon(painter = painterResource(R.drawable.ic_trashcan), contentDescription = null, modifier = Modifier.size(16.dp))
                            Spacer(Modifier.width(4.dp))
                            Text("Delete", fontSize = 13.sp)
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
                JustifiedGrid(
                    items = viewModel.items,
                    getAspectRatio = { item ->
                        if (item.width > 0 && item.height > 0) item.width.toFloat() / item.height.toFloat()
                        else 1f
                    },
                    getKey = { it.id },
                    targetRowHeight = 180.dp,
                    gap = 2.dp,
                ) { item, widthDp, heightDp ->
                    TrashTile(
                        item = item,
                        serverBaseUrl = viewModel.serverBaseUrl,
                        decryptedThumbPath = viewModel.decryptedThumbPaths[item.id],
                        isSelectionMode = viewModel.isSelectionMode,
                        isSelected = item.id in viewModel.selectedIds,
                        onLongPress = { viewModel.enterSelectionMode(item.id) },
                        onTap = {
                            if (viewModel.isSelectionMode) {
                                viewModel.toggleSelect(item.id)
                            }
                        },
                        widthDp = widthDp,
                        heightDp = heightDp,
                    )
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
    decryptedThumbPath: String?,
    isSelectionMode: Boolean,
    isSelected: Boolean,
    onLongPress: () -> Unit,
    onTap: () -> Unit,
    widthDp: Dp,
    heightDp: Dp,
) {
    // For encrypted items, use the locally decrypted thumbnail file.
    // For unencrypted items, load from the server trash thumbnail endpoint.
    val thumbSource: Any = if (decryptedThumbPath != null) {
        java.io.File(decryptedThumbPath)
    } else {
        "$serverBaseUrl/api/trash/${item.id}/thumb"
    }

    Box(
        modifier = Modifier
            .width(widthDp)
            .height(heightDp)
            .clip(MaterialTheme.shapes.small)
            .combinedClickable(
                onClick = onTap,
                onLongClick = onLongPress
            )
    ) {
        // Thumbnail
        AsyncImage(
            model = ImageRequest.Builder(LocalContext.current)
                .data(thumbSource)
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
