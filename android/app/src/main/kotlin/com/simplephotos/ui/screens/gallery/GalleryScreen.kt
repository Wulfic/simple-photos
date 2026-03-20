/**
 * Main photo gallery screen — displays a responsive grid of photos/videos
 * with multi-select support, pull-to-refresh, upload from device, and
 * long-press to enter selection mode for batch delete/share/album operations.
 */
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
import androidx.compose.ui.graphics.Brush
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
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.*
import javax.inject.Inject
import com.simplephotos.R
import androidx.compose.ui.res.painterResource

// ── Screen ──────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun GalleryScreen(
    onPhotoClick: (String) -> Unit,
    onAlbumsClick: () -> Unit,
    onSearchClick: () -> Unit,
    onTrashClick: () -> Unit,
    onSettingsClick: () -> Unit,
    onSecureGalleryClick: () -> Unit = {},
    onSharedAlbumsClick: () -> Unit = {},
    onDiagnosticsClick: () -> Unit = {},
    onLogout: () -> Unit,
    isAdmin: Boolean = false,
    viewModel: GalleryViewModel = hiltViewModel()
) {
    val photos by viewModel.photos.collectAsState(initial = emptyList())
    val albums by viewModel.albums.collectAsState(initial = emptyList())
    val context = LocalContext.current
    var showAlbumPicker by remember { mutableStateOf(false) }
    val isSystemDark = androidx.compose.foundation.isSystemInDarkTheme()

    // Filter out photos that live in a secure gallery
    val visiblePhotos = remember(photos, viewModel.secureBlobIds) {
        if (viewModel.secureBlobIds.isEmpty()) photos
        else photos.filter { it.serverBlobId == null || it.serverBlobId !in viewModel.secureBlobIds }
    }

    // Build day-grouped grid items
    val gridItems = remember(visiblePhotos) { buildGridItems(groupPhotosByDay(visiblePhotos)) }

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
                                onClick = { viewModel.selectAll(visiblePhotos) },
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
                                onClick = { viewModel.deleteSelectedPhotos(visiblePhotos) },
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
                        onSecureGalleryClick = onSecureGalleryClick,
                        onSharedAlbumsClick = onSharedAlbumsClick,
                        onDiagnosticsClick = onDiagnosticsClick,
                        onLogout = { viewModel.logout(onLogout) },
                        onToggleTheme = { ThemeState.toggle(viewModel.dataStore, ThemeState.isDark(isSystemDark)) },
                        isAdmin = isAdmin
                    ),
                    isSyncing = viewModel.isSyncing,
                    syncLabel = if (viewModel.isSyncing) "Syncing" else null
                )
            }
        },
        floatingActionButton = {
            if (!viewModel.isSelectionMode) {
                FloatingActionButton(
                    onClick = { pickMediaLauncher.launch(arrayOf("image/*", "video/*", "audio/*")) },
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
                // Wait for sync to actually complete before ending the refresh indicator
                snapshotFlow { viewModel.isSyncing }
                    .filter { !it }
                    .first()
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

                if (visiblePhotos.isEmpty() && !viewModel.isSyncing && viewModel.dataReady) {
                    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text("No photos yet", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Spacer(Modifier.height(8.dp))
                            Text("Tap + to add photos or grant permissions for auto-backup", textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(horizontal = 32.dp))
                        }
                    }
                } else if (!viewModel.dataReady || (visiblePhotos.isEmpty() && viewModel.isSyncing)) {
                    // Show loading until the first server sync completes.
                    // This prevents flashing stale photos from a previous user session.
                    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            CircularProgressIndicator()
                            Spacer(Modifier.height(16.dp))
                            Text("Syncing from server...")
                        }
                    }
                } else {
                    // Day-grouped photo grid
                    // "large" = fewer columns (bigger thumbnails), "normal" = more columns
                    val gridMinSize = if (viewModel.thumbnailSize == "large") 160.dp else 100.dp
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(gridMinSize),
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
            // Filename overlay — only shown for audio files (images/videos rely on visual thumbnail)
            if (photo.mediaType == "audio") {
                Box(
                    modifier = Modifier
                        .align(Alignment.BottomStart)
                        .fillMaxWidth()
                        .background(
                            Brush.verticalGradient(
                                colors = listOf(Color.Transparent, Color.Black.copy(alpha = 0.6f))
                            )
                        )
                        .padding(start = 4.dp, end = 4.dp, top = 12.dp, bottom = 2.dp)
                ) {
                    Text(
                        text = photo.filename,
                        color = Color.White,
                        fontSize = 8.sp,
                        maxLines = 1,
                        overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis
                    )
                }
            }
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
        } else if (photo.mediaType == "audio") {
            Surface(modifier = Modifier.align(Alignment.BottomEnd).padding(4.dp), shape = MaterialTheme.shapes.extraSmall, color = Color.Black.copy(alpha = 0.6f)) {
                Text(
                    text = if (photo.durationSecs != null) { val m = (photo.durationSecs / 60).toInt(); val s = (photo.durationSecs % 60).toInt(); "\u266B $m:${s.toString().padStart(2, '0')}" } else "\u266B",
                    color = Color.White, fontSize = 10.sp, modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp)
                )
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
