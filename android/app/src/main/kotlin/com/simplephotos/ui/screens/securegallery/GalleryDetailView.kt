package com.simplephotos.ui.screens.securegallery

import android.graphics.BitmapFactory
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items as lazyItems
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.simplephotos.data.collapseBursts
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SecureGalleryItem
import com.simplephotos.ui.screens.album.AlbumRemoval
import com.simplephotos.ui.screens.album.SecureRemovalVerdict
import com.simplephotos.ui.theme.Violet

// ─────────────────────────────────────────────────────────────────────────────
// Gallery Detail View
// ─────────────────────────────────────────────────────────────────────────────

/** Top-level source tabs in the "Add Photos" picker (mirrors the web flow). */
private enum class PickerTab { All, Recents, Albums }

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
internal fun GalleryDetailView(
    title: String,
    items: List<SecureGalleryItem>,
    itemsLoading: Boolean,
    error: String?,
    onBack: () -> Unit,
    onAddPhotos: (List<String>) -> Unit,
    viewModel: SecureGalleryViewModel,
    // Read-only smart-album view: no target album to add into, so every "Add
    // Photos" affordance is hidden. Per-item Remove still works (routed to the
    // item's real owning album by the view model).
    readOnly: Boolean = false,
) {
    var showAddPhotos by remember { mutableStateOf(false) }
    var selectedBlobIds by remember { mutableStateOf(emptySet<String>()) }
    // Internal viewer state — show secure items only, not the main gallery
    var viewerIndex by remember { mutableStateOf<Int?>(null) }

    // Cross-secure-album move picker (#31): pull media in from the user's OTHER
    // secure albums. A photo lives in one secure album, so this is a move.
    var showMovePicker by remember { mutableStateOf(false) }
    var moveSelectedIds by remember { mutableStateOf(emptySet<String>()) }
    val otherSecureItems = viewModel.otherSecureItems

    // ── Add-photos picker source state ──────────────────────────────────────
    var pickerTab by remember { mutableStateOf(PickerTab.All) }
    // When the Albums tab has an album open, this is its id (else browse list).
    var browsingAlbumId by remember { mutableStateOf<String?>(null) }

    // ── Grid multi-select (for deleting) state ──────────────────────────────
    var selectionMode by remember { mutableStateOf(false) }
    var selectedItemIds by remember { mutableStateOf(emptySet<String>()) }
    // Items the user has ASKED to remove, awaiting the confirmation below (Z1e).
    // Held as state rather than removed inline because the question is now
    // conditional: what removal does depends on how many OTHER secure albums
    // hold each photo, and one of the three answers is "we don't know, so don't
    // ask". Also fed by the viewer, so a removal requested from the pager can be
    // declined without losing the page.
    var pendingRemoval by remember { mutableStateOf<List<SecureGalleryItem>?>(null) }
    // Push (#43, an ADD since Z1): pick another secure album for the selection.
    var showMoveTarget by remember { mutableStateOf(false) }
    // Exiting selection mode whenever the album's items change keeps the bar
    // honest (e.g. after a delete the selection is gone).
    fun exitSelection() { selectionMode = false; selectedItemIds = emptySet() }

    // Picker source: full library ("all") or a specific album / smart album.
    val pickerPhotos = viewModel.pickerPhotos
    val pickerAlbums = viewModel.pickerAlbums
    val secureBlobIds = viewModel.secureBlobIds
    // Picker excludes anything already secured (in ANY secure album — same id
    // set the main gallery hides), then collapses bursts so the user picks one
    // tile per burst (matching the main gallery picker).
    val availablePhotos = remember(pickerPhotos, secureBlobIds) {
        pickerPhotos.filter {
            it.serverBlobId != null &&
                it.serverBlobId !in secureBlobIds &&
                (it.serverPhotoId == null || it.serverPhotoId !in secureBlobIds)
        }.collapseBursts()
    }
    // Burst frame counts (from the UN-collapsed source) so a picker burst cover
    // can badge "BURST n" the same way the album grid does.
    val pickerBurstCounts = remember(pickerPhotos) {
        pickerPhotos.mapNotNull { it.burstId }.filter { it.isNotEmpty() }
            .groupingBy { it }.eachCount()
    }

    // Collapse burst stacks → one tile / pager page per burst. The album still
    // physically holds every frame; we just display the cover. Counts come from
    // the FULL list so the tile can badge "BURST n".
    val displayItems = remember(items) { collapseSecureBursts(items) }
    val burstCounts = remember(items) {
        items.mapNotNull { it.burstId }.filter { it.isNotEmpty() }
            .groupingBy { it }.eachCount()
    }

    // ── Removal confirmation (Z1e) ──────────────────────────────────────────
    //
    // Emitted BEFORE the viewer's early return on purpose: an AlertDialog is its
    // own window, so this renders over the pager too, and a removal requested
    // from the viewer can be cancelled without closing it.
    //
    // Until Z1e this dialog said "The selected photos will return to your
    // regular gallery" unconditionally, and the viewer's remove had no
    // confirmation at all. Since Z1 a photo may live in several secure albums,
    // so that sentence is false whenever another album still holds it — the user
    // is told they un-secured something they did not. The verdict now comes from
    // `AlbumRemoval`, shared with web, and has a third arm: refuse when the
    // server did not publish membership, because guessing is the Z1 bug
    // restated.
    pendingRemoval?.let { targets ->
        // The SAME expansion `removeItems` will apply, so the prompt counts the
        // batch that is actually removed rather than the tiles that were tapped.
        val expanded = SecureMovePlan.expandForRemoval(items, targets)
        val memberships = expanded.map { item ->
            // No owning album id means we cannot even name the membership set to
            // count, which is unknown — not zero.
            val owning = item.galleryId ?: viewModel.selectedGallery?.id
            if (owning == null) null
            else AlbumRemoval.otherSecureAlbumCount(item.galleries, owning)
        }
        // The album NAME must come from the OWNING album, not the open view: in
        // a smart view `title` is "Videos", and naming that as the thing being
        // removed from is wrong in the one dialog whose whole job is accuracy. A
        // batch spanning several albums names none rather than the wrong one.
        val owningName =
            if (!readOnly) title
            else expanded.mapNotNull { it.galleryName }.distinct().singleOrNull()
        val verdict = AlbumRemoval.secureRemovalPrompt(memberships, owningName)
        AlertDialog(
            onDismissRequest = { pendingRemoval = null },
            title = { Text(verdict.prompt.title) },
            text = { Text(verdict.prompt.body) },
            confirmButton = {
                when (verdict) {
                    is SecureRemovalVerdict.Confirm -> TextButton(onClick = {
                        pendingRemoval = null
                        viewModel.removeItems(targets)
                        exitSelection()
                        viewerIndex = null
                    }) { Text("Remove") }
                    // Recovery, not just a refusal — re-fetching the feeds is the
                    // only thing that can resolve the unknown.
                    is SecureRemovalVerdict.Blocked -> TextButton(onClick = {
                        pendingRemoval = null
                        viewModel.refreshFeeds()
                    }) { Text("Refresh") }
                }
            },
            dismissButton = {
                TextButton(onClick = { pendingRemoval = null }) { Text("Cancel") }
            }
        )
    }

    // Full-screen viewer for secure items only
    if (viewerIndex != null) {
        SecurePhotoViewer(
            items = displayItems,
            allItems = items,
            initialIndex = viewerIndex!!,
            viewModel = viewModel,
            onBack = { viewerIndex = null },
            onRemove = { item ->
                // Ask first (Z1e). Burst-aware: the confirmation and the removal
                // share one expansion, so a burst cover asks about the stack.
                pendingRemoval = listOf(item)
            }
        )
        return
    }

    // Target-album picker for the push direction (#43, an ADD since Z1).
    if (showMoveTarget) {
        val targets = viewModel.addTargets
        AlertDialog(
            onDismissRequest = { showMoveTarget = false },
            title = { Text("Add ${selectedItemIds.size} to") },
            text = {
                if (targets.isEmpty()) {
                    Text("No other secure albums to add to.")
                } else {
                    Column(
                        modifier = Modifier
                            .heightIn(max = 320.dp)
                            .verticalScroll(rememberScrollState())
                    ) {
                        // Z1: this ADDS. Said plainly, because the same control
                        // used to move, and a user who learned the old behaviour
                        // will otherwise assume it empties the current album.
                        Text(
                            "${if (selectedItemIds.size != 1) "They stay" else "It stays"} in " +
                                "“$title” as well.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(bottom = 8.dp)
                        )
                        targets.forEach { g ->
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable {
                                        viewModel.pushItemsTo(selectedItemIds.toList(), g.id)
                                        showMoveTarget = false
                                        exitSelection()
                                    }
                                    .padding(vertical = 12.dp, horizontal = 4.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(10.dp)
                            ) {
                                Icon(
                                    Icons.Default.Folder,
                                    contentDescription = null,
                                    modifier = Modifier.size(20.dp),
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                                Text(g.name, maxLines = 1)
                            }
                        }
                    }
                }
            },
            confirmButton = {},
            dismissButton = {
                TextButton(onClick = { showMoveTarget = false }) { Text("Cancel") }
            }
        )
    }

    Scaffold(
        topBar = {
            if (selectionMode) {
                // Contextual selection bar — count + delete, X to exit.
                TopAppBar(
                    title = { Text("${selectedItemIds.size} selected", maxLines = 1) },
                    navigationIcon = {
                        IconButton(onClick = { exitSelection() }) {
                            Icon(Icons.Default.Close, contentDescription = "Cancel selection")
                        }
                    },
                    actions = {
                        // Select all / deselect all the items in this album (#25),
                        // matching the regular gallery's multi-select affordance.
                        val allSelected = displayItems.isNotEmpty() &&
                            displayItems.all { it.id in selectedItemIds }
                        TextButton(onClick = {
                            selectedItemIds = if (allSelected) emptySet()
                                else displayItems.map { it.id }.toSet()
                            if (selectedItemIds.isEmpty()) selectionMode = false
                        }) {
                            Text(if (allSelected) "Deselect all" else "Select all")
                        }
                        // File the selection into another secure album (#43) —
                        // only when there is somewhere to add it to. The verb is
                        // "Add", not "Move": since Z1 the photo stays here too,
                        // and "Move" is the verb this item was filed to kill.
                        if (viewModel.addTargets.isNotEmpty()) {
                            TextButton(
                                onClick = { if (selectedItemIds.isNotEmpty()) showMoveTarget = true },
                                enabled = selectedItemIds.isNotEmpty()
                            ) {
                                Text("Add to album")
                            }
                        }
                        IconButton(
                            onClick = {
                                if (selectedItemIds.isNotEmpty()) {
                                    pendingRemoval = displayItems.filter { it.id in selectedItemIds }
                                }
                            },
                            enabled = selectedItemIds.isNotEmpty()
                        ) {
                            Icon(Icons.Default.Delete, contentDescription = "Remove selected")
                        }
                    }
                )
            } else {
                TopAppBar(
                    title = {
                        Column {
                            Text(title, maxLines = 1)
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
                        if (!readOnly && !showAddPhotos && !showMovePicker) {
                            // Move in from other secure albums (#31) — only when
                            // there is somewhere to pull from.
                            if (otherSecureItems.isNotEmpty()) {
                                TextButton(onClick = {
                                    showMovePicker = true
                                    moveSelectedIds = emptySet()
                                }) { Text("From secure") }
                            }
                            IconButton(onClick = {
                                showAddPhotos = true
                                selectedBlobIds = emptySet()
                                pickerTab = PickerTab.All
                                browsingAlbumId = null
                                viewModel.selectPickerSource("all")
                            }) {
                                Icon(Icons.Default.Add, contentDescription = "Add Photos")
                            }
                        }
                    }
                )
            }
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

            if (showMovePicker) {
                MoveFromSecurePanel(
                    items = otherSecureItems,
                    selectedIds = moveSelectedIds,
                    viewModel = viewModel,
                    onToggle = { id ->
                        moveSelectedIds = if (id in moveSelectedIds)
                            moveSelectedIds - id else moveSelectedIds + id
                    },
                    onMove = {
                        viewModel.moveItemsIntoSelected(moveSelectedIds.toList())
                        showMovePicker = false
                        moveSelectedIds = emptySet()
                    },
                    onCancel = {
                        showMovePicker = false
                        moveSelectedIds = emptySet()
                    },
                )
            } else if (showAddPhotos) {
                AddPhotosPanel(
                    availablePhotos = availablePhotos,
                    pickerAlbums = pickerAlbums,
                    pickerBurstCounts = pickerBurstCounts,
                    pickerTab = pickerTab,
                    browsingAlbumId = browsingAlbumId,
                    selectedBlobIds = selectedBlobIds,
                    onSelectTab = { tab ->
                        pickerTab = tab
                        browsingAlbumId = null
                        when (tab) {
                            PickerTab.All -> viewModel.selectPickerSource("all")
                            PickerTab.Recents -> viewModel.selectPickerSource("smart-recents")
                            PickerTab.Albums -> { /* show browser; no source change yet */ }
                        }
                    },
                    onOpenAlbum = { albumId ->
                        browsingAlbumId = albumId
                        viewModel.selectPickerSource(albumId)
                    },
                    onCloseAlbum = { browsingAlbumId = null },
                    onToggle = { blobId ->
                        selectedBlobIds = if (blobId in selectedBlobIds)
                            selectedBlobIds - blobId else selectedBlobIds + blobId
                    },
                    onSelectAll = {
                        // Select-all toggles the *current source's* photos into the
                        // running selection (which persists across tabs/albums), so
                        // adding a whole album to a secure gallery is one tap (#25).
                        val ids = availablePhotos.mapNotNull { it.serverBlobId }.toSet()
                        val allSelected = ids.isNotEmpty() && selectedBlobIds.containsAll(ids)
                        selectedBlobIds = if (allSelected)
                            selectedBlobIds - ids else selectedBlobIds + ids
                    },
                    onAdd = {
                        // Expand burst representatives to their full stack
                        // (pickerPhotos is the un-collapsed source).
                        onAddPhotos(expandBurstBlobIds(selectedBlobIds, pickerPhotos))
                        showAddPhotos = false
                        selectedBlobIds = emptySet()
                    },
                    onCancel = {
                        showAddPhotos = false
                        selectedBlobIds = emptySet()
                    }
                )
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
                            if (!readOnly) {
                                Spacer(Modifier.height(8.dp))
                                Button(onClick = {
                                    showAddPhotos = true
                                    selectedBlobIds = emptySet()
                                    pickerTab = PickerTab.All
                                    browsingAlbumId = null
                                    viewModel.selectPickerSource("all")
                                }) {
                                    Text("Add Photos")
                                }
                            }
                        }
                    }
                } else {
                    com.simplephotos.ui.components.JustifiedGrid(
                        items = displayItems,
                        getAspectRatio = { it ->
                            // Crop-effective aspect (#31) so a cropped tile lays
                            // out at the right shape and FillBounds shows cleanly.
                            secureEffectiveAspect(it.width, it.height, it.cropMetadata)
                        },
                        getKey = { it.id },
                        targetRowHeight = com.simplephotos.ui.components.rememberGalleryRowHeight(),
                        gap = 2.dp,
                    ) { item, widthDp, heightDp ->
                        val isSelected = item.id in selectedItemIds
                        Box(
                            modifier = Modifier
                                .size(widthDp, heightDp)
                                .combinedClickable(
                                    onClick = {
                                        if (selectionMode) {
                                            selectedItemIds = if (isSelected)
                                                selectedItemIds - item.id
                                            else selectedItemIds + item.id
                                            // Leaving nothing selected exits the mode.
                                            if (selectedItemIds.isEmpty()) selectionMode = false
                                        } else {
                                            viewerIndex = displayItems.indexOfFirst { it.id == item.id }
                                                .coerceAtLeast(0)
                                        }
                                    },
                                    onLongClick = {
                                        selectionMode = true
                                        selectedItemIds = selectedItemIds + item.id
                                    }
                                )
                        ) {
                            SecureItemTile(
                                item = item,
                                burstCount = item.burstId?.let { burstCounts[it] } ?: 0,
                                viewModel = viewModel
                            )
                            if (selectionMode) {
                                if (isSelected) {
                                    Box(
                                        modifier = Modifier
                                            .fillMaxSize()
                                            .background(Violet.v500.copy(alpha = 0.35f))
                                    )
                                }
                                Surface(
                                    modifier = Modifier
                                        .align(Alignment.TopEnd)
                                        .padding(6.dp)
                                        .size(22.dp),
                                    shape = androidx.compose.foundation.shape.CircleShape,
                                    color = if (isSelected) Violet.v600 else Color.Black.copy(alpha = 0.35f),
                                    border = androidx.compose.foundation.BorderStroke(1.5.dp, Color.White)
                                ) {
                                    if (isSelected) {
                                        Box(contentAlignment = Alignment.Center) {
                                            Text("✓", color = Color.White, fontSize = 13.sp)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Add Photos panel — source tabs + album browser + picker grid
// ─────────────────────────────────────────────────────────────────────────────

@OptIn(ExperimentalFoundationApi::class, ExperimentalMaterial3Api::class)
@Composable
private fun ColumnScope.AddPhotosPanel(
    availablePhotos: List<PhotoEntity>,
    pickerAlbums: List<PickerAlbum>,
    pickerBurstCounts: Map<String, Int>,
    pickerTab: PickerTab,
    browsingAlbumId: String?,
    selectedBlobIds: Set<String>,
    onSelectTab: (PickerTab) -> Unit,
    onOpenAlbum: (String) -> Unit,
    onCloseAlbum: () -> Unit,
    onToggle: (String) -> Unit,
    onSelectAll: () -> Unit,
    onAdd: () -> Unit,
    onCancel: () -> Unit,
) {
    // Selection action bar.
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.3f))
            .padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                "Selected ${selectedBlobIds.size}",
                style = MaterialTheme.typography.bodyMedium
            )
            // Select all / deselect all the current source's photos (#25).
            val ids = availablePhotos.mapNotNull { it.serverBlobId }
            val allSelected = ids.isNotEmpty() && selectedBlobIds.containsAll(ids)
            TextButton(
                onClick = onSelectAll,
                enabled = ids.isNotEmpty(),
                contentPadding = PaddingValues(horizontal = 8.dp)
            ) {
                Text(if (allSelected) "Deselect all" else "Select all")
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = onAdd, enabled = selectedBlobIds.isNotEmpty()) { Text("Add") }
            OutlinedButton(onClick = onCancel) { Text("Cancel") }
        }
    }

    // Source tabs: All Photos | Recently Added | Albums (mirrors the web flow).
    androidx.compose.foundation.lazy.LazyRow(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
    ) {
        item {
            FilterChip(
                selected = pickerTab == PickerTab.All,
                onClick = { onSelectTab(PickerTab.All) },
                label = { Text("All Photos") }
            )
        }
        item {
            FilterChip(
                selected = pickerTab == PickerTab.Recents,
                onClick = { onSelectTab(PickerTab.Recents) },
                label = { Text("Recently Added") }
            )
        }
        item {
            FilterChip(
                selected = pickerTab == PickerTab.Albums,
                onClick = { onSelectTab(PickerTab.Albums) },
                label = { Text("Albums") }
            )
        }
    }

    if (pickerTab == PickerTab.Albums && browsingAlbumId == null) {
        // Album browser — pick which album to add from (like the web Albums menu).
        if (pickerAlbums.isEmpty()) {
            Box(
                modifier = Modifier.fillMaxWidth().padding(32.dp),
                contentAlignment = Alignment.Center
            ) {
                Text("No albums yet.", color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        } else {
            LazyColumn(modifier = Modifier.weight(1f)) {
                lazyItems(pickerAlbums, key = { it.id }) { album ->
                    AlbumBrowserRow(album = album, onClick = { onOpenAlbum(album.id) })
                }
            }
        }
        return
    }

    // Picker grid (for All / Recently Added / a chosen album).
    if (pickerTab == PickerTab.Albums && browsingAlbumId != null) {
        val name = pickerAlbums.firstOrNull { it.id == browsingAlbumId }?.name ?: "Album"
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(onClick = onCloseAlbum)
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back to albums",
                modifier = Modifier.size(18.dp))
            Text(name, style = MaterialTheme.typography.titleSmall, maxLines = 1)
        }
    }

    if (availablePhotos.isEmpty()) {
        Box(
            modifier = Modifier.fillMaxWidth().padding(32.dp),
            contentAlignment = Alignment.Center
        ) {
            Text("No photos available to add.", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    } else {
        // Aspect-correct justified grid (matches the album/gallery grids) so
        // wide/tall photos read correctly while picking.
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
                            if (blobId != null) onToggle(blobId)
                        }
                ) {
                    PhotoThumbnail(photo)
                    // Subtype / media badges (BURST n, LIVE, 360°, PANO, video…).
                    MediaTileBadges(
                        mediaType = photo.mediaType,
                        photoSubtype = photo.photoSubtype,
                        burstId = photo.burstId,
                        burstCount = photo.burstId?.let { pickerBurstCounts[it] } ?: 0,
                        durationSecs = photo.durationSecs,
                    )
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
}

// ─────────────────────────────────────────────────────────────────────────────
// Move-from-secure panel — pick items from the user's OTHER secure albums (#31)
// ─────────────────────────────────────────────────────────────────────────────

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ColumnScope.MoveFromSecurePanel(
    items: List<SecureGalleryItem>,
    selectedIds: Set<String>,
    viewModel: SecureGalleryViewModel,
    onToggle: (String) -> Unit,
    onMove: () -> Unit,
    onCancel: () -> Unit,
) {
    // Collapse bursts so one tile represents a stack (the move pulls the cover;
    // the server keeps membership rows per frame — bursts move as their cover
    // here for parity with the add/remove flows' single-tile display).
    val display = remember(items) { collapseSecureBursts(items) }
    val burstCounts = remember(items) {
        items.mapNotNull { it.burstId }.filter { it.isNotEmpty() }.groupingBy { it }.eachCount()
    }

    // Action bar.
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.3f))
            .padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text("Selected ${selectedIds.size}", style = MaterialTheme.typography.bodyMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = onMove, enabled = selectedIds.isNotEmpty()) { Text("Move here") }
            OutlinedButton(onClick = onCancel) { Text("Cancel") }
        }
    }

    if (display.isEmpty()) {
        Box(
            modifier = Modifier.fillMaxWidth().padding(32.dp),
            contentAlignment = Alignment.Center
        ) {
            Text("No other secure albums to move from.", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        return
    }

    Box(modifier = Modifier.weight(1f)) {
        com.simplephotos.ui.components.JustifiedGrid(
            items = display,
            getAspectRatio = { secureEffectiveAspect(it.width, it.height, it.cropMetadata) },
            getKey = { it.id },
            targetRowHeight = com.simplephotos.ui.components.rememberGalleryRowHeight(),
            gap = 2.dp,
        ) { item, widthDp, heightDp ->
            val isSelected = item.id in selectedIds
            Box(
                modifier = Modifier
                    .size(widthDp, heightDp)
                    .clip(RoundedCornerShape(4.dp))
                    .clickable { onToggle(item.id) }
            ) {
                SecureItemTile(
                    item = item,
                    burstCount = item.burstId?.let { burstCounts[it] } ?: 0,
                    viewModel = viewModel,
                )
                if (isSelected) {
                    Box(modifier = Modifier.fillMaxSize().background(Violet.v500.copy(alpha = 0.3f)))
                    Surface(
                        modifier = Modifier.align(Alignment.TopEnd).padding(4.dp).size(20.dp),
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

/** One album row in the secure add-photos "Albums" browser. */
@Composable
private fun AlbumBrowserRow(album: PickerAlbum, onClick: () -> Unit) {
    val cover = remember(album.coverThumbPath) {
        album.coverThumbPath?.let { try { BitmapFactory.decodeFile(it) } catch (_: Exception) { null } }
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Box(
            modifier = Modifier
                .size(48.dp)
                .clip(RoundedCornerShape(8.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant),
            contentAlignment = Alignment.Center
        ) {
            if (cover != null) {
                Image(
                    bitmap = cover.asImageBitmap(),
                    contentDescription = null,
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Crop
                )
            } else {
                Icon(
                    Icons.Default.Folder,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
        Text(album.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1)
    }
}

/** Collapse burst stacks in secure items: keep the first frame per burstId. */
internal fun collapseSecureBursts(items: List<SecureGalleryItem>): List<SecureGalleryItem> {
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
