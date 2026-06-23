package com.simplephotos.ui.screens.securegallery

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.items as lazyItems
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.simplephotos.data.collapseBursts
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem
import com.simplephotos.ui.theme.Violet

// ─────────────────────────────────────────────────────────────────────────────
// Gallery Detail View
// ─────────────────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun GalleryDetailView(
    gallery: SecureGallery,
    items: List<SecureGalleryItem>,
    itemsLoading: Boolean,
    error: String?,
    onBack: () -> Unit,
    onAddPhotos: (List<String>) -> Unit,
    viewModel: SecureGalleryViewModel
) {
    var showAddPhotos by remember { mutableStateOf(false) }
    var selectedBlobIds by remember { mutableStateOf(emptySet<String>()) }
    // Internal viewer state — show secure items only, not the main gallery
    var viewerIndex by remember { mutableStateOf<Int?>(null) }

    val albumBlobIds = remember(items) { items.map { it.blobId }.toSet() }
    // Picker source: full library ("all") or a specific album / smart album.
    val pickerPhotos = viewModel.pickerPhotos
    val pickerAlbums = viewModel.pickerAlbums
    val pickerSourceId = viewModel.pickerSourceId
    // Picker excludes anything already in the album, then collapses bursts so
    // the user picks one tile per burst (matching the main gallery picker).
    val availablePhotos = remember(pickerPhotos, albumBlobIds) {
        pickerPhotos.filter { it.serverBlobId != null && it.serverBlobId !in albumBlobIds }
            .collapseBursts()
    }

    // Collapse burst stacks → one tile / pager page per burst. The album still
    // physically holds every frame; we just display the cover. Counts come from
    // the FULL list so the tile can badge "BURST n".
    val displayItems = remember(items) { collapseSecureBursts(items) }
    val burstCounts = remember(items) {
        items.mapNotNull { it.burstId }.filter { it.isNotEmpty() }
            .groupingBy { it }.eachCount()
    }

    // Full-screen viewer for secure items only
    if (viewerIndex != null) {
        SecurePhotoViewer(
            items = displayItems,
            initialIndex = viewerIndex!!,
            viewModel = viewModel,
            onBack = { viewerIndex = null },
            onRemove = { item ->
                viewModel.removeItem(item)
                viewerIndex = null
            }
        )
        return
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(gallery.name, maxLines = 1)
                        Text(
                            "${displayItems.size} items",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    if (!showAddPhotos) {
                        IconButton(onClick = {
                            showAddPhotos = true
                            selectedBlobIds = emptySet()
                            viewModel.selectPickerSource("all")
                        }) {
                            Icon(Icons.Default.Add, contentDescription = "Add Photos")
                        }
                    }
                }
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
        ) {
            error?.let {
                Text(
                    it,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp)
                )
            }

            // Add photos bar
            if (showAddPhotos) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.3f))
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        "Select photos (${selectedBlobIds.size})",
                        style = MaterialTheme.typography.bodyMedium
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(
                            onClick = {
                                // Expand burst representatives to their full stack
                                // (pickerPhotos is the un-collapsed source).
                                onAddPhotos(expandBurstBlobIds(selectedBlobIds, pickerPhotos))
                                showAddPhotos = false
                                selectedBlobIds = emptySet()
                            },
                            enabled = selectedBlobIds.isNotEmpty()
                        ) { Text("Add") }
                        OutlinedButton(onClick = {
                            showAddPhotos = false
                            selectedBlobIds = emptySet()
                        }) { Text("Cancel") }
                    }
                }

                // Source selector — pick from the whole library or a specific
                // album / smart album (parity with the web album-based flow).
                if (pickerAlbums.isNotEmpty()) {
                    androidx.compose.foundation.lazy.LazyRow(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
                    ) {
                        item {
                            FilterChip(
                                selected = pickerSourceId == "all",
                                onClick = { viewModel.selectPickerSource("all") },
                                label = { Text("All Photos") }
                            )
                        }
                        lazyItems(pickerAlbums) { (id, name) ->
                            FilterChip(
                                selected = pickerSourceId == id,
                                onClick = { viewModel.selectPickerSource(id) },
                                label = { Text(name, maxLines = 1) }
                            )
                        }
                    }
                }

                // Photo picker grid
                if (availablePhotos.isEmpty()) {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(32.dp),
                        contentAlignment = Alignment.Center
                    ) {
                        Text(
                            "No photos available to add.",
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                } else {
                    // Aspect-correct justified grid (matches the album/gallery grids)
                    // instead of fixed squares, so wide/tall photos read correctly
                    // while picking. Wrapped in a weighted Box so the grid's
                    // LazyColumn fills the space under the "Select photos" bar.
                    Box(modifier = Modifier.weight(1f)) {
                        com.simplephotos.ui.components.JustifiedGrid(
                            items = availablePhotos,
                            getAspectRatio = { p ->
                                if (p.width > 0 && p.height > 0) p.width.toFloat() / p.height.toFloat() else 1f
                            },
                            getKey = { it.localId },
                            targetRowHeight = com.simplephotos.ui.components.rememberGalleryRowHeight(),
                            gap = 2.dp,
                        ) { photo, widthDp, heightDp ->
                            val blobId = photo.serverBlobId
                            val isSelected = blobId != null && blobId in selectedBlobIds
                            Box(
                                modifier = Modifier
                                    .size(widthDp, heightDp)
                                    .clip(RoundedCornerShape(4.dp))
                                    .clickable(enabled = blobId != null) {
                                        if (blobId != null) {
                                            selectedBlobIds = if (isSelected)
                                                selectedBlobIds - blobId
                                            else
                                                selectedBlobIds + blobId
                                        }
                                    }
                            ) {
                                PhotoThumbnail(photo)
                                if (isSelected) {
                                    Box(
                                        modifier = Modifier
                                            .fillMaxSize()
                                            .background(Violet.v500.copy(alpha = 0.3f))
                                    )
                                    Surface(
                                        modifier = Modifier
                                            .align(Alignment.TopEnd)
                                            .padding(4.dp)
                                            .size(20.dp),
                                        shape = androidx.compose.foundation.shape.CircleShape,
                                        color = Violet.v600
                                    ) {
                                        Box(contentAlignment = Alignment.Center) {
                                            Text("✓", color = Color.White, fontSize = 12.sp)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                // Items grid
                if (itemsLoading) {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center
                    ) {
                        CircularProgressIndicator()
                    }
                } else if (items.isEmpty()) {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center
                    ) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text("This album is empty.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Spacer(Modifier.height(8.dp))
                            Button(onClick = {
                                showAddPhotos = true
                                selectedBlobIds = emptySet()
                                viewModel.selectPickerSource("all")
                            }) {
                                Text("Add Photos")
                            }
                        }
                    }
                } else {
                    com.simplephotos.ui.components.JustifiedGrid(
                        items = displayItems,
                        getAspectRatio = { it ->
                            val w = it.width ?: 0
                            val h = it.height ?: 0
                            if (w > 0 && h > 0) w.toFloat() / h.toFloat() else 1f
                        },
                        getKey = { it.id },
                        targetRowHeight = com.simplephotos.ui.components.rememberGalleryRowHeight(),
                        gap = 2.dp,
                    ) { item, widthDp, heightDp ->
                        val openViewer = {
                            val idx = displayItems.indexOfFirst { it.id == item.id }
                            viewerIndex = idx.coerceAtLeast(0)
                        }
                        Box(
                            modifier = Modifier
                                .size(widthDp, heightDp)
                                .clickable { openViewer() }
                        ) {
                            SecureItemTile(
                                item = item,
                                burstCount = item.burstId?.let { burstCounts[it] } ?: 0,
                                onClick = openViewer,
                                viewModel = viewModel
                            )
                        }
                    }
                }
            }
        }
    }
}

/** Collapse burst stacks in secure items: keep the first frame per burstId. */
private fun collapseSecureBursts(items: List<SecureGalleryItem>): List<SecureGalleryItem> {
    val seen = HashSet<String>()
    return items.filter { item ->
        val bid = item.burstId
        if (bid.isNullOrEmpty()) true else seen.add(bid)
    }
}

/**
 * Expand a set of selected server blob IDs so that any selected burst
 * representative also pulls in the rest of its burst frames. The picker grid
 * collapses bursts to one tile, so a selection only holds the cover frame's
 * blobId — without this, only the cover would move into the secure album.
 *
 * [allPhotos] is the un-collapsed picker source, so it still carries every
 * burst frame; non-burst selections pass through unchanged.
 */
internal fun expandBurstBlobIds(
    selected: Set<String>,
    allPhotos: List<PhotoEntity>,
): List<String> {
    if (selected.isEmpty()) return emptyList()
    val byBlob = allPhotos.filter { it.serverBlobId != null }.associateBy { it.serverBlobId!! }
    val burstIds = selected.mapNotNull { byBlob[it]?.burstId }
        .filter { it.isNotEmpty() }
        .toSet()
    if (burstIds.isEmpty()) return selected.toList()
    val members = allPhotos
        .filter { !it.burstId.isNullOrEmpty() && it.burstId in burstIds && it.serverBlobId != null }
        .mapNotNull { it.serverBlobId }
    return (selected + members).distinct()
}
