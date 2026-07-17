package com.simplephotos.ui.screens.securegallery

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items as lazyGridItems
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.simplephotos.R
import com.simplephotos.data.SecureSmartAlbum
import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem

// ─────────────────────────────────────────────────────────────────────────────
// Gallery List View
// ─────────────────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun GalleryListView(
    galleries: List<SecureGallery>,
    galleriesLoading: Boolean,
    error: String?,
    onBack: () -> Unit,
    onGalleryClick: (SecureGallery) -> Unit,
    onSmartAlbumClick: (SecureSmartAlbum) -> Unit,
    onCreateGallery: (String) -> Unit,
    onDeleteGallery: (SecureGallery) -> Unit,
    viewModel: SecureGalleryViewModel
) {
    // Built-in smart albums (non-empty types only), derived from the aggregate
    // feed. Read-only: no delete affordance, no create.
    val smartAlbums = viewModel.smartAlbums
    var showCreate by remember { mutableStateOf(false) }
    var newName by remember { mutableStateOf("") }
    var confirmDeleteId by remember { mutableStateOf<String?>(null) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Secure Albums") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    IconButton(onClick = { showCreate = !showCreate }) {
                        Icon(Icons.Default.Add, contentDescription = "New Album")
                    }
                }
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 16.dp)
        ) {
            // Create form
            if (showCreate) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    OutlinedTextField(
                        value = newName,
                        onValueChange = { if (it.length <= 100) newName = it },
                        placeholder = { Text("Album name") },
                        singleLine = true,
                        modifier = Modifier.weight(1f)
                    )
                    Spacer(Modifier.width(8.dp))
                    Button(
                        onClick = {
                            if (newName.isNotBlank()) {
                                onCreateGallery(newName.trim())
                                newName = ""
                                showCreate = false
                            }
                        },
                        enabled = newName.isNotBlank()
                    ) { Text("Create") }
                }
            }

            error?.let {
                Text(
                    it,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(vertical = 4.dp)
                )
            }

            if (galleriesLoading) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    CircularProgressIndicator()
                }
            } else if (galleries.isEmpty() && smartAlbums.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Icon(
                            painter = painterResource(R.drawable.ic_locks),
                            contentDescription = null,
                            modifier = Modifier.size(48.dp),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(Modifier.height(8.dp))
                        Text(
                            "No secure albums yet",
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Text(
                            "Create an album to store your most private photos.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center
                        )
                        if (!showCreate) {
                            Spacer(Modifier.height(12.dp))
                            Button(onClick = { showCreate = true }) {
                                Text("+ Create Album")
                            }
                        }
                    }
                }
            } else {
                // 2-column card grid mirroring the regular Albums screen, with
                // the delete action tucked inside each card (top-right) the way
                // the album cards present their in-card actions.
                LazyVerticalGrid(
                    columns = GridCells.Fixed(2),
                    modifier = Modifier.fillMaxSize(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                    contentPadding = PaddingValues(vertical = 8.dp)
                ) {
                    // Built-in smart albums first (full-width header + cards),
                    // then the user's own albums. Smart cards have no delete.
                    if (smartAlbums.isNotEmpty()) {
                        item(span = { GridItemSpan(maxLineSpan) }) {
                            Text(
                                "Smart albums",
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = FontWeight.Medium,
                                modifier = Modifier.padding(top = 4.dp, bottom = 4.dp)
                            )
                        }
                        lazyGridItems(smartAlbums, key = { it.id }) { sa ->
                            SmartAlbumCard(
                                smartAlbum = sa,
                                viewModel = viewModel,
                                onClick = { onSmartAlbumClick(sa) }
                            )
                        }
                    }
                    if (galleries.isNotEmpty() && smartAlbums.isNotEmpty()) {
                        item(span = { GridItemSpan(maxLineSpan) }) {
                            Text(
                                "Albums",
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = FontWeight.Medium,
                                modifier = Modifier.padding(top = 8.dp, bottom = 4.dp)
                            )
                        }
                    }
                    lazyGridItems(galleries, key = { it.id }) { gallery ->
                        Card(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { onGalleryClick(gallery) },
                            shape = RoundedCornerShape(12.dp)
                        ) {
                            Box {
                                Column(modifier = Modifier.padding(12.dp)) {
                                    GalleryCoverThumbnail(
                                        galleryId = gallery.id,
                                        itemCount = gallery.itemCount,
                                        viewModel = viewModel,
                                        modifier = Modifier
                                            .fillMaxWidth()
                                            .aspectRatio(1f)
                                    )
                                    Spacer(Modifier.height(8.dp))
                                    Text(
                                        gallery.name,
                                        style = MaterialTheme.typography.titleSmall,
                                        fontWeight = FontWeight.Medium,
                                        maxLines = 1
                                    )
                                    Text(
                                        "${gallery.itemCount} item${if (gallery.itemCount != 1) "s" else ""}",
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant
                                    )
                                }
                                Surface(
                                    onClick = {
                                        if (confirmDeleteId == gallery.id) {
                                            onDeleteGallery(gallery)
                                            confirmDeleteId = null
                                        } else {
                                            confirmDeleteId = gallery.id
                                        }
                                    },
                                    modifier = Modifier
                                        .align(Alignment.TopEnd)
                                        .padding(8.dp)
                                        .size(32.dp),
                                    shape = CircleShape,
                                    color = Color.Black.copy(alpha = 0.45f)
                                ) {
                                    Box(contentAlignment = Alignment.Center) {
                                        Icon(
                                            Icons.Default.Delete,
                                            contentDescription = if (confirmDeleteId == gallery.id)
                                                "Tap again to confirm delete" else "Delete",
                                            tint = if (confirmDeleteId == gallery.id)
                                                MaterialTheme.colorScheme.error
                                            else
                                                Color.White,
                                            modifier = Modifier.size(18.dp)
                                        )
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
// Smart Album Card — a built-in, read-only secure smart album (no delete)
// ─────────────────────────────────────────────────────────────────────────────

@Composable
private fun SmartAlbumCard(
    smartAlbum: SecureSmartAlbum,
    viewModel: SecureGalleryViewModel,
    onClick: () -> Unit,
) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        shape = RoundedCornerShape(12.dp)
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            SmartAlbumCoverThumbnail(
                item = smartAlbum.coverItem,
                viewModel = viewModel,
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(1f)
            )
            Spacer(Modifier.height(8.dp))
            Text(
                smartAlbum.label,
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Medium,
                maxLines = 1
            )
            Text(
                "${smartAlbum.count} item${if (smartAlbum.count != 1) "s" else ""}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

/**
 * Cover thumbnail for a smart album, rendered from an ALREADY-KNOWN item (the
 * cover from the aggregate feed) — no extra `listItems` fetch, unlike
 * [GalleryCoverThumbnail].
 */
@Composable
private fun SmartAlbumCoverThumbnail(
    item: SecureGalleryItem,
    viewModel: SecureGalleryViewModel,
    modifier: Modifier = Modifier.size(48.dp)
) {
    var bitmap by remember(item.blobId) { mutableStateOf<android.graphics.Bitmap?>(null) }

    LaunchedEffect(item.blobId) {
        if (bitmap == null) {
            val data = try {
                viewModel.downloadThumb(item.blobId, item.encryptedThumbBlobId)
            } catch (_: Exception) { null }
            if (data != null) {
                bitmap = try {
                    BitmapFactory.decodeByteArray(data, 0, data.size)
                } catch (_: Exception) { null }
            }
        }
    }

    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(8.dp),
        color = MaterialTheme.colorScheme.primaryContainer
    ) {
        val bmp = bitmap
        if (bmp != null) {
            Image(
                bitmap = bmp.asImageBitmap(),
                contentDescription = "Album cover",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
        } else {
            Box(contentAlignment = Alignment.Center) {
                Icon(
                    painter = painterResource(R.drawable.ic_locks),
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary
                )
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Gallery Cover Thumbnail — decrypted preview of a secure album's newest item
// ─────────────────────────────────────────────────────────────────────────────

@Composable
internal fun GalleryCoverThumbnail(
    galleryId: String,
    itemCount: Int,
    viewModel: SecureGalleryViewModel,
    modifier: Modifier = Modifier.size(48.dp)
) {
    var bitmap by remember(galleryId) { mutableStateOf<android.graphics.Bitmap?>(null) }

    // Fetch the newest item's decrypted thumbnail as the album cover. Lazy:
    // only galleries actually rendered (and non-empty) trigger a fetch.
    LaunchedEffect(galleryId, itemCount) {
        if (itemCount > 0 && bitmap == null) {
            val data = viewModel.fetchGalleryCover(galleryId)
            if (data != null) {
                bitmap = try {
                    BitmapFactory.decodeByteArray(data, 0, data.size)
                } catch (_: Exception) { null }
            }
        }
    }

    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(8.dp),
        color = MaterialTheme.colorScheme.primaryContainer
    ) {
        val bmp = bitmap
        if (bmp != null) {
            Image(
                bitmap = bmp.asImageBitmap(),
                contentDescription = "Album cover",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
        } else {
            Box(contentAlignment = Alignment.Center) {
                Icon(
                    painter = painterResource(R.drawable.ic_locks),
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary
                )
            }
        }
    }
}
