/**
 * Composable screen that displays the user's albums in a scrollable list
 * with options to create, rename, and delete albums. Shared albums are
 * rendered inline at the bottom — matching the web Albums page layout.
 */
package com.simplephotos.ui.screens.album

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
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
import androidx.hilt.navigation.compose.hiltViewModel
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.AlbumEntity
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SharedAlbumInfo
import com.simplephotos.ui.components.ActiveTab
import com.simplephotos.ui.components.AppHeader
import com.simplephotos.ui.components.HeaderNavigation
import com.simplephotos.ui.theme.ThemeState

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
    onPeople: () -> Unit = {},
    onPets: () -> Unit = {},
    onMemories: () -> Unit = {},
    onTrips: () -> Unit = {},
    onPersonClick: (Long) -> Unit = {},
    onPetClick: (Long) -> Unit = {},
    onMemoryClick: (String) -> Unit = {},
    onTripClick: (String) -> Unit = {},
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

            // Build combined list: smart albums pinned at top, then user albums.
            // Audio only appears when there is at least one audio item, matching
            // the web behaviour (web/src/pages/Albums.tsx renders the Audio card
            // only when encryptedPhotoCounts.audio > 0).
            val smartAlbumEntries = buildList {
                add(Triple("smart-favorites", "Favorites", viewModel.favoritesCount))
                add(Triple("smart-photos", "Photos", viewModel.photosCount))
                add(Triple("smart-gifs", "GIFs", viewModel.gifsCount))
                add(Triple("smart-videos", "Videos", viewModel.videosCount))
                if (viewModel.audioCount > 0) {
                    add(Triple("smart-audio", "Audio", viewModel.audioCount))
                }
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
                                        onClick = { onAlbumClick(item.id) }
                                    )
                                }
                                is AlbumGridItem.UserAlbum -> {
                                    AlbumCard(
                                        album = item.album,
                                        coverPhoto = viewModel.albumCoverPhotos[item.album.localId],
                                        serverBaseUrl = viewModel.serverBaseUrl,
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

            // ── Discover (smart) album sections ─ horizontal rows per category ──
            val anyDiscover = viewModel.peopleClusters.isNotEmpty() ||
                viewModel.petClusters.isNotEmpty() ||
                viewModel.memories.isNotEmpty() ||
                viewModel.trips.isNotEmpty()
            if (anyDiscover) {
                Spacer(Modifier.height(16.dp))
                HorizontalDivider(
                    modifier = Modifier.padding(horizontal = 16.dp),
                    color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f)
                )
                Text(
                    "Discover",
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)
                )
            }

            if (viewModel.peopleClusters.isNotEmpty()) {
                DiscoverSection(
                    title = "People",
                    overflowCap = 6,
                    items = viewModel.peopleClusters,
                    keyOf = { it.id.toString() },
                    primary = { it.label ?: "Unnamed" },
                    secondary = { "${it.photoCount} photos" },
                    thumbUrl = { c ->
                        c.representative?.let { id -> "${viewModel.serverBaseUrl}/api/photos/$id/thumb" }
                    },
                    onHeaderClick = onPeople,
                    onItemClick = { cluster -> onPersonClick(cluster.id) },
                    onOverflowClick = onPeople,
                )
            }
            if (viewModel.petClusters.isNotEmpty()) {
                DiscoverSection(
                    title = "Pets",
                    overflowCap = 6,
                    items = viewModel.petClusters,
                    keyOf = { it.id.toString() },
                    primary = { it.label ?: it.species },
                    secondary = { "${it.photoCount} photos" },
                    thumbUrl = { c ->
                        c.representative?.let { id -> "${viewModel.serverBaseUrl}/api/photos/$id/thumb" }
                    },
                    onHeaderClick = onPets,
                    onItemClick = { cluster -> onPetClick(cluster.id) },
                    onOverflowClick = onPets,
                )
            }
            if (viewModel.memories.isNotEmpty()) {
                DiscoverSection(
                    title = "Memories",
                    overflowCap = 4,
                    items = viewModel.memories,
                    keyOf = { it.id },
                    primary = { it.name },
                    secondary = { it.dateLabel },
                    thumbUrl = { m ->
                        m.firstPhotoId?.let { id -> "${viewModel.serverBaseUrl}/api/photos/$id/thumb" }
                    },
                    onHeaderClick = onMemories,
                    onItemClick = { memory -> onMemoryClick(memory.id) },
                    onOverflowClick = onMemories,
                )
            }
            if (viewModel.trips.isNotEmpty()) {
                DiscoverSection(
                    title = "Trips",
                    overflowCap = 4,
                    items = viewModel.trips,
                    keyOf = { it.id },
                    primary = { it.city },
                    secondary = { it.dateLabel },
                    thumbUrl = { t ->
                        t.firstPhotoId?.let { id -> "${viewModel.serverBaseUrl}/api/photos/$id/thumb" }
                    },
                    onHeaderClick = onTrips,
                    onItemClick = { trip -> onTripClick(trip.id) },
                    onOverflowClick = onTrips,
                )
            }

            // Always show the 4-card icon shortcut row (matches the web's
            // "open the dedicated screen" affordance even when no clusters
            // exist yet).
            if (!anyDiscover) {
                Spacer(Modifier.height(16.dp))
                HorizontalDivider(
                    modifier = Modifier.padding(horizontal = 16.dp),
                    color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f)
                )
                Text(
                    "Discover",
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)
                )
                val discoverEntries = listOf<Triple<String, Int, () -> Unit>>(
                    Triple("People", com.simplephotos.R.drawable.ic_shared, onPeople),
                    Triple("Pets", com.simplephotos.R.drawable.ic_image, onPets),
                    Triple("Memories", com.simplephotos.R.drawable.ic_star, onMemories),
                    Triple("Trips", com.simplephotos.R.drawable.ic_folder, onTrips),
                )
                discoverEntries.chunked(2).forEach { rowItems ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 4.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        rowItems.forEach { (label, iconRes, onClick) ->
                            Box(modifier = Modifier.weight(1f)) {
                                DiscoverCard(label = label, iconRes = iconRes, onClick = onClick)
                            }
                        }
                        if (rowItems.size == 1) {
                            Spacer(Modifier.weight(1f))
                        }
                    }
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

/**
 * Renders a "Discover" smart-section card (People, Pets, Things, Map, etc.)
 * Tapping the card navigates into the corresponding library sub-screen.
 */
@Composable
private fun DiscoverCard(
    label: String,
    iconRes: Int,
    onClick: () -> Unit = {}
) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        shape = MaterialTheme.shapes.medium
    ) {
        Column {
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
                                    Color(0xFF8B5CF6).copy(alpha = 0.15f),
                                    Color(0xFF3B82F6).copy(alpha = 0.15f)
                                )
                            )
                        ),
                    contentAlignment = Alignment.Center
                ) {
                    Icon(
                        painter = androidx.compose.ui.res.painterResource(iconRes),
                        contentDescription = label,
                        modifier = Modifier.size(48.dp),
                        tint = MaterialTheme.colorScheme.primary
                    )
                }
            }
            Column(modifier = Modifier.padding(8.dp)) {
                Text(
                    label,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1
                )
            }
        }
    }
}

/**
 * Horizontal section showing the first N items as cards, followed by a
 * "See K more" overflow card when the list exceeds [overflowCap]. Mirrors
 * the layout used by web/src/pages/Albums.tsx for People / Pets / Memories
 * / Trips sections.
 */
@Composable
private fun <T> DiscoverSection(
    title: String,
    overflowCap: Int,
    items: List<T>,
    keyOf: (T) -> String,
    primary: (T) -> String,
    secondary: (T) -> String,
    thumbUrl: (T) -> String?,
    onHeaderClick: () -> Unit,
    onItemClick: (T) -> Unit,
    onOverflowClick: () -> Unit,
) {
    val total = items.size
    // Web pattern: if total > overflowCap, show first (overflowCap - 1) cards
    // plus an overflow card. Otherwise show all.
    val visible = if (total > overflowCap) items.take(overflowCap - 1) else items
    val overflowCount = if (total > overflowCap) total - visible.size else 0

    Spacer(Modifier.height(8.dp))
    // The section title itself acts as a link to the full subpage —
    // matches the web Albums.tsx behaviour (the <h2> word is the link).
    Text(
        title,
        style = MaterialTheme.typography.titleMedium,
        fontWeight = FontWeight.SemiBold,
        modifier = Modifier
            .padding(horizontal = 16.dp, vertical = 4.dp)
            .clickable(onClick = onHeaderClick)
    )
    LazyRow(
        modifier = Modifier.fillMaxWidth(),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        items(visible, key = { keyOf(it) }) { item ->
            DiscoverThumbCard(
                primary = primary(item),
                secondary = secondary(item),
                thumbUrl = thumbUrl(item),
                onClick = { onItemClick(item) },
            )
        }
        if (overflowCount > 0) {
            item(key = "$title-overflow") {
                DiscoverOverflowCard(
                    label = "See $overflowCount more",
                    onClick = onOverflowClick,
                )
            }
        }
    }
}

@Composable
private fun DiscoverThumbCard(
    primary: String,
    secondary: String,
    thumbUrl: String?,
    onClick: () -> Unit,
) {
    Card(
        modifier = Modifier
            .width(120.dp)
            .clickable(onClick = onClick),
        shape = MaterialTheme.shapes.medium
    ) {
        Column {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(1f)
                    .background(
                        Brush.linearGradient(
                            listOf(
                                Color(0xFF8B5CF6).copy(alpha = 0.15f),
                                Color(0xFF3B82F6).copy(alpha = 0.15f)
                            )
                        )
                    ),
                contentAlignment = Alignment.Center
            ) {
                if (!thumbUrl.isNullOrEmpty()) {
                    AsyncImage(
                        model = ImageRequest.Builder(LocalContext.current)
                            .data(thumbUrl)
                            .crossfade(true)
                            .build(),
                        contentDescription = primary,
                        contentScale = ContentScale.Crop,
                        modifier = Modifier.fillMaxSize()
                    )
                } else {
                    Icon(
                        Icons.Default.Person,
                        contentDescription = null,
                        modifier = Modifier.size(36.dp),
                        tint = MaterialTheme.colorScheme.primary
                    )
                }
            }
            Column(modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp)) {
                Text(
                    primary,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1
                )
                Text(
                    secondary,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1
                )
            }
        }
    }
}

@Composable
private fun DiscoverOverflowCard(
    label: String,
    onClick: () -> Unit,
) {
    Card(
        modifier = Modifier
            .width(120.dp)
            .clickable(onClick = onClick),
        shape = MaterialTheme.shapes.medium
    ) {
        Column {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(1f)
                    .background(
                        Brush.linearGradient(
                            listOf(
                                Color(0xFF8B5CF6).copy(alpha = 0.25f),
                                Color(0xFF3B82F6).copy(alpha = 0.25f)
                            )
                        )
                    ),
                contentAlignment = Alignment.Center
            ) {
                Icon(
                    Icons.Default.Add,
                    contentDescription = null,
                    modifier = Modifier.size(40.dp),
                    tint = MaterialTheme.colorScheme.primary
                )
            }
            Column(modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp)) {
                // Two text rows so the card has identical height to DiscoverThumbCard
                // (which renders both a primary and secondary label).
                Text(
                    label,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1
                )
                Text(
                    "See all",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1
                )
            }
        }
    }
}
