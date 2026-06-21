@file:OptIn(ExperimentalMaterial3Api::class)

package com.simplephotos.ui.screens.securegallery

import android.graphics.BitmapFactory
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items as lazyItems
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import coil.compose.AsyncImage
import coil.request.ImageRequest
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import com.simplephotos.ui.theme.Violet
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.simplephotos.R
import com.simplephotos.data.collapseBursts
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem
import com.simplephotos.ui.screens.viewer.MAX_PANO_DECODE_PX
import com.simplephotos.ui.screens.viewer.PanoramaOverlay
import android.net.Uri
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

// ─────────────────────────────────────────────────────────────────────────────
// Password Gate
// ─────────────────────────────────────────────────────────────────────────────

@Composable
internal fun PasswordGate(
    onBack: () -> Unit,
    onUnlock: (String) -> Unit,
    isLoading: Boolean,
    error: String?,
    canUseBiometric: Boolean = false,
    onBiometricRequest: () -> Unit = {}
) {
    var password by remember { mutableStateOf("") }
    var passwordVisible by remember { mutableStateOf(false) }

    // Auto-prompt biometric on first display when stored credentials exist
    LaunchedEffect(canUseBiometric) {
        if (canUseBiometric) {
            onBiometricRequest()
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Secure Albums") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                }
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center
        ) {
            Icon(
                painter = painterResource(R.drawable.ic_locks),
                contentDescription = null,
                modifier = Modifier.size(64.dp),
                tint = MaterialTheme.colorScheme.primary
            )
            Spacer(Modifier.height(16.dp))
            Text(
                "Secure Albums",
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.Bold
            )
            Text(
                if (canUseBiometric)
                    "Use biometrics or enter your password to access your secure albums."
                else
                    "Enter your account password to access your secure albums.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(top = 8.dp, bottom = 24.dp)
            )

            // Biometric button — shown prominently when credentials are stored
            if (canUseBiometric) {
                OutlinedButton(
                    onClick = onBiometricRequest,
                    modifier = Modifier.fillMaxWidth(),
                    enabled = !isLoading
                ) {
                    Icon(
                        imageVector = Icons.Default.Fingerprint,
                        contentDescription = null,
                        modifier = Modifier.size(20.dp)
                    )
                    Spacer(Modifier.width(8.dp))
                    Text("Use Biometrics")
                }
                Spacer(Modifier.height(16.dp))
                Text(
                    "or enter password",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(12.dp))
            }

            OutlinedTextField(
                value = password,
                onValueChange = { password = it },
                label = { Text("Password") },
                visualTransformation = if (passwordVisible) VisualTransformation.None else PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Password,
                    autoCorrect = false
                ),
                trailingIcon = {
                    IconButton(onClick = { passwordVisible = !passwordVisible }) {
                        Icon(
                            imageVector = if (passwordVisible) Icons.Default.VisibilityOff else Icons.Default.Visibility,
                            contentDescription = if (passwordVisible) "Hide password" else "Show password"
                        )
                    }
                },
                singleLine = true,
                modifier = Modifier.fillMaxWidth()
            )

            error?.let {
                Text(
                    it,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(top = 8.dp)
                )
            }

            Spacer(Modifier.height(16.dp))

            Button(
                onClick = { onUnlock(password) },
                enabled = password.isNotEmpty() && !isLoading,
                modifier = Modifier.fillMaxWidth()
            ) {
                if (isLoading) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(20.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary
                    )
                } else {
                    Text("Unlock")
                }
            }
        }
    }
}

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
    onCreateGallery: (String) -> Unit,
    onDeleteGallery: (SecureGallery) -> Unit,
    viewModel: SecureGalleryViewModel
) {
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
            } else if (galleries.isEmpty()) {
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
                LazyColumn(
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                    contentPadding = PaddingValues(vertical = 8.dp)
                ) {
                    lazyItems(galleries, key = { it.id }) { gallery ->
                        Card(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { onGalleryClick(gallery) },
                            shape = RoundedCornerShape(12.dp)
                        ) {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(16.dp),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                GalleryCoverThumbnail(
                                    galleryId = gallery.id,
                                    itemCount = gallery.itemCount,
                                    viewModel = viewModel
                                )
                                Spacer(Modifier.width(12.dp))
                                Column(modifier = Modifier.weight(1f)) {
                                    Text(
                                        gallery.name,
                                        style = MaterialTheme.typography.titleSmall,
                                        fontWeight = FontWeight.Medium
                                    )
                                    Text(
                                        "${gallery.itemCount} item${if (gallery.itemCount != 1) "s" else ""}",
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant
                                    )
                                }
                                IconButton(
                                    onClick = {
                                        if (confirmDeleteId == gallery.id) {
                                            onDeleteGallery(gallery)
                                            confirmDeleteId = null
                                        } else {
                                            confirmDeleteId = gallery.id
                                        }
                                    }
                                ) {
                                    Icon(
                                        Icons.Default.Delete,
                                        contentDescription = "Delete",
                                        tint = if (confirmDeleteId == gallery.id)
                                            MaterialTheme.colorScheme.error
                                        else
                                            MaterialTheme.colorScheme.onSurfaceVariant
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

// ─────────────────────────────────────────────────────────────────────────────
// Gallery Detail View
// ─────────────────────────────────────────────────────────────────────────────

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun GalleryDetailView(
    gallery: SecureGallery,
    items: List<SecureGalleryItem>,
    itemsLoading: Boolean,
    allPhotos: List<PhotoEntity>,
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
    // Picker excludes anything already in the album, then collapses bursts so
    // the user picks one tile per burst (matching the main gallery picker).
    val availablePhotos = remember(allPhotos, albumBlobIds) {
        allPhotos.filter { it.serverBlobId != null && it.serverBlobId !in albumBlobIds }
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
                        IconButton(onClick = { showAddPhotos = true; selectedBlobIds = emptySet() }) {
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
                                onAddPhotos(selectedBlobIds.toList())
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
                            targetRowHeight = 110.dp,
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
                            Button(onClick = { showAddPhotos = true; selectedBlobIds = emptySet() }) {
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
                        targetRowHeight = 130.dp,
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

// ─────────────────────────────────────────────────────────────────────────────
// Gallery Cover Thumbnail — decrypted preview of a secure album's newest item
// ─────────────────────────────────────────────────────────────────────────────

@Composable
internal fun GalleryCoverThumbnail(
    galleryId: String,
    itemCount: Int,
    viewModel: SecureGalleryViewModel
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
        modifier = Modifier.size(48.dp),
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
// Secure Item Tile — downloads and shows decrypted thumbnail
// ─────────────────────────────────────────────────────────────────────────────

@Composable
internal fun SecureItemTile(
    item: SecureGalleryItem,
    onClick: () -> Unit,
    viewModel: SecureGalleryViewModel,
    burstCount: Int = 0
) {
    var bitmap by remember(item.blobId) { mutableStateOf<android.graphics.Bitmap?>(null) }
    var gifBytes by remember(item.blobId) { mutableStateOf<ByteArray?>(null) }
    var loading by remember(item.blobId) { mutableStateOf(true) }

    LaunchedEffect(item.blobId) {
        loading = true
        try {
            val data = viewModel.downloadThumb(item.blobId, item.encryptedThumbBlobId)
            // Detect GIF by magic bytes (GIF87a / GIF89a)
            val isGif = data.size > 3 &&
                data[0] == 0x47.toByte() && data[1] == 0x49.toByte() && data[2] == 0x46.toByte()
            if (isGif) {
                gifBytes = data
            } else {
                bitmap = BitmapFactory.decodeByteArray(data, 0, data.size)
            }
        } catch (e: Exception) {
            android.util.Log.e("SecureItemTile", "Failed to load thumb for blobId=${item.blobId}", e)
            bitmap = null
            gifBytes = null
        } finally {
            loading = false
        }
    }

    Box(
        modifier = Modifier
            .aspectRatio(1f)
            .clip(RoundedCornerShape(4.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center
    ) {
        when {
            loading -> CircularProgressIndicator(
                modifier = Modifier.size(24.dp),
                strokeWidth = 2.dp
            )
            gifBytes != null -> AsyncImage(
                model = ImageRequest.Builder(LocalContext.current)
                    .data(java.nio.ByteBuffer.wrap(gifBytes!!))
                    .build(),
                contentDescription = "Secure photo",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
            bitmap != null -> Image(
                bitmap = bitmap!!.asImageBitmap(),
                contentDescription = "Secure photo",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
            else -> {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(
                        painter = painterResource(R.drawable.ic_locks),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(24.dp)
                    )
                    Text(
                        "Encrypted",
                        fontSize = 10.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }

        // ── Subtype / media badges (mirror the main gallery tiles) ──────────
        val sub = item.photoSubtype
        val durLabel = item.durationSecs?.let { d ->
            val m = (d / 60).toInt(); val s = (d % 60).toInt(); "$m:${s.toString().padStart(2, '0')}"
        }
        when (item.mediaType) {
            "video" -> SecureTileBadge("▶" + (durLabel?.let { " $it" } ?: ""), Alignment.BottomStart)
            "gif" -> SecureTileBadge("GIF", Alignment.BottomStart)
            "audio" -> SecureTileBadge("♫" + (durLabel?.let { " $it" } ?: ""), Alignment.BottomStart)
        }
        val topLabel = when {
            sub == "equirectangular" -> "360°"
            sub == "panorama" -> "PANO"
            sub == "motion" -> "LIVE"
            burstCount > 1 -> "BURST $burstCount"
            !item.burstId.isNullOrEmpty() -> "BURST"
            else -> null
        }
        if (topLabel != null) SecureTileBadge(topLabel, Alignment.TopStart, bold = true)
    }
}

/** Small translucent badge used on secure tiles (matches the gallery style). */
@Composable
private fun BoxScope.SecureTileBadge(
    text: String,
    alignment: Alignment,
    bold: Boolean = false
) {
    Surface(
        modifier = Modifier.align(alignment).padding(4.dp),
        shape = MaterialTheme.shapes.extraSmall,
        color = Color.Black.copy(alpha = 0.6f)
    ) {
        Text(
            text = text,
            color = Color.White,
            fontSize = if (bold) 9.sp else 10.sp,
            fontWeight = if (bold) FontWeight.Bold else FontWeight.Normal,
            modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp)
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Photo Thumbnail helper
// ─────────────────────────────────────────────────────────────────────────────

@Composable
internal fun PhotoThumbnail(photo: PhotoEntity) {
    val isGif = photo.mediaType == "gif"
    when {
        isGif && photo.localPath != null -> {
            AsyncImage(
                model = ImageRequest.Builder(LocalContext.current)
                    .data(android.net.Uri.parse(photo.localPath))
                    .build(),
                contentDescription = photo.filename,
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
        }
        photo.thumbnailPath != null -> {
            val bitmap = remember(photo.thumbnailPath) {
                try { BitmapFactory.decodeFile(photo.thumbnailPath) } catch (_: Exception) { null }
            }
            bitmap?.let {
                Image(
                    bitmap = it.asImageBitmap(),
                    contentDescription = photo.filename,
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Crop
                )
            }
        }
        else -> {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .background(MaterialTheme.colorScheme.surfaceVariant),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    photo.filename.take(6),
                    fontSize = 9.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center
                )
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Secure Photo Viewer — full-screen pager for encrypted items only
// ─────────────────────────────────────────────────────────────────────────────

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun SecurePhotoViewer(
    items: List<SecureGalleryItem>,
    initialIndex: Int,
    viewModel: SecureGalleryViewModel,
    onBack: () -> Unit,
    onRemove: ((SecureGalleryItem) -> Unit)? = null
) {
    val pagerState = rememberPagerState(
        initialPage = initialIndex.coerceIn(0, (items.size - 1).coerceAtLeast(0)),
        pageCount = { items.size }
    )
    var confirmRemove by remember { mutableStateOf(false) }
    // When a panorama / 360 page enters Live (pan) mode we must stop the pager
    // from stealing the horizontal drag (otherwise panning flips pages). Reset
    // whenever the page changes so a swipe away always re-enables paging.
    var panoLive by remember { mutableStateOf(false) }
    LaunchedEffect(pagerState.currentPage) { panoLive = false }

    if (confirmRemove) {
        val current = items.getOrNull(pagerState.currentPage)
        AlertDialog(
            onDismissRequest = { confirmRemove = false },
            title = { Text("Remove from secure album?") },
            text = { Text("The photo will return to your regular gallery.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmRemove = false
                    current?.let { onRemove?.invoke(it) }
                }) { Text("Remove") }
            },
            dismissButton = {
                TextButton(onClick = { confirmRemove = false }) { Text("Cancel") }
            }
        )
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black)
    ) {
        HorizontalPager(
            state = pagerState,
            userScrollEnabled = !panoLive,
            modifier = Modifier.fillMaxSize()
        ) { page ->
            SecureMediaPage(
                item = items[page],
                viewModel = viewModel,
                onPanoLiveModeChange = { live ->
                    if (pagerState.currentPage == page) panoLive = live
                }
            )
        }

        // Back button overlay
        IconButton(
            onClick = onBack,
            modifier = Modifier
                .statusBarsPadding()
                .padding(8.dp)
                .align(Alignment.TopStart)
        ) {
            Icon(
                Icons.AutoMirrored.Filled.ArrowBack,
                contentDescription = "Back",
                tint = Color.White
            )
        }

        // Remove-from-album overlay (mirrors web's per-item removal)
        if (onRemove != null && items.isNotEmpty()) {
            IconButton(
                onClick = { confirmRemove = true },
                modifier = Modifier
                    .statusBarsPadding()
                    .padding(8.dp)
                    .align(Alignment.TopEnd)
            ) {
                Icon(
                    Icons.Default.Delete,
                    contentDescription = "Remove from album",
                    tint = Color.White
                )
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Secure media page — type-aware renderer for one pager page
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Renders one secure item full-screen, branching on its type so the secure
 * viewer matches the main gallery:
 *   - video    → decrypt to a temp file, play with ExoPlayer + controls
 *   - pano/360 → still image + interactive [PanoramaOverlay] (reused from viewer)
 *   - motion   → still image + LIVE overlay (embedded MP4 extracted client-side)
 *   - photo/gif→ Coil image (Coil sniffs GIF / AVIF / etc.)
 *
 * Image types are decrypted to a ByteArray and handed to Coil, which downsamples
 * safely (panoramas capped to [MAX_PANO_DECODE_PX] to dodge the "too large
 * bitmap" crash). Videos / motion trailers go to disk and are wiped on dispose
 * so the decrypted plaintext doesn't linger in the cache.
 */
@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun SecureMediaPage(
    item: SecureGalleryItem,
    viewModel: SecureGalleryViewModel,
    onPanoLiveModeChange: (Boolean) -> Unit,
) {
    val sub = item.photoSubtype
    val isVideo = item.mediaType == "video"
    val isPano = sub == "panorama" || sub == "equirectangular"
    val isMotion = sub == "motion" && !isVideo

    if (isVideo) {
        SecureVideoPage(item, viewModel)
        return
    }

    val context = LocalContext.current
    var decrypted by remember(item.blobId) { mutableStateOf<ByteArray?>(null) }
    var loading by remember(item.blobId) { mutableStateOf(true) }
    var failed by remember(item.blobId) { mutableStateOf(false) }

    LaunchedEffect(item.blobId) {
        loading = true; failed = false
        try {
            decrypted = viewModel.downloadAndDecrypt(item.blobId)
        } catch (e: Exception) {
            android.util.Log.e("SecureMediaPage", "decrypt failed blobId=${item.blobId}", e)
            failed = true
        } finally {
            loading = false
        }
    }

    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        when {
            loading -> CircularProgressIndicator(color = Color.White)
            failed || decrypted == null -> Text("Failed to decrypt", color = Color.White)
            else -> {
                val data = decrypted!!
                AsyncImage(
                    model = ImageRequest.Builder(context)
                        .data(data)
                        .apply {
                            // Capped decode (NOT ORIGINAL) for wide panos/360 — see MAX_PANO_DECODE_PX.
                            if (isPano) { size(MAX_PANO_DECODE_PX); allowHardware(false) }
                        }
                        .crossfade(true)
                        .build(),
                    contentDescription = "Secure photo",
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Fit
                )

                if (isPano) {
                    PanoramaOverlay(
                        imageData = data,
                        intrinsicWidth = (item.width ?: 0).toFloat(),
                        intrinsicHeight = (item.height ?: 0).toFloat(),
                        is360 = sub == "equirectangular",
                        contentDescription = "Secure panorama",
                        onLiveModeChange = { live, _ -> onPanoLiveModeChange(live) },
                    )
                } else if (isMotion) {
                    SecureMotionOverlay(jpegBytes = data, blobKey = item.blobId)
                }
            }
        }
    }
}

/**
 * Plays a decrypted secure video. The blob is streamed-decrypted to a temp file
 * (ExoPlayer needs a file/URI, not a ByteArray) and deleted on dispose.
 */
@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun SecureVideoPage(
    item: SecureGalleryItem,
    viewModel: SecureGalleryViewModel,
) {
    val context = LocalContext.current
    var videoFile by remember(item.blobId) { mutableStateOf<File?>(null) }
    var loading by remember(item.blobId) { mutableStateOf(true) }
    var failed by remember(item.blobId) { mutableStateOf(false) }

    LaunchedEffect(item.blobId) {
        loading = true; failed = false
        try {
            videoFile = viewModel.downloadAndDecryptToFile(item.blobId, "mp4")
        } catch (e: Exception) {
            android.util.Log.e("SecureVideoPage", "decrypt video failed blobId=${item.blobId}", e)
            failed = true
        } finally {
            loading = false
        }
    }

    // Wipe the decrypted plaintext when leaving the page (confidentiality).
    DisposableEffect(videoFile) {
        val f = videoFile
        onDispose { f?.delete() }
    }

    val player = remember(videoFile) {
        videoFile?.let { f ->
            ExoPlayer.Builder(context).build().apply {
                setMediaItem(MediaItem.fromUri(Uri.fromFile(f)))
                prepare()
                playWhenReady = false
            }
        }
    }
    DisposableEffect(player) { onDispose { player?.release() } }

    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        when {
            loading -> CircularProgressIndicator(color = Color.White)
            failed || player == null -> Text("Unable to play this video", color = Color.White)
            else -> AndroidView(
                modifier = Modifier.fillMaxSize(),
                factory = { ctx ->
                    PlayerView(ctx).apply {
                        this.player = player
                        useController = true
                    }
                }
            )
        }
    }
}

/**
 * Plays the motion-photo trailer embedded inside a decrypted JPEG, muted and
 * looping, on top of the still. The MP4 is extracted client-side (the secure
 * clone has no separate motion-video blob) and wiped on dispose. Renders
 * nothing extra if no embedded video is found — the still already shows.
 */
@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun SecureMotionOverlay(
    jpegBytes: ByteArray,
    blobKey: String,
) {
    val context = LocalContext.current
    var videoFile by remember(blobKey) { mutableStateOf<File?>(null) }
    var available by remember(blobKey) { mutableStateOf(true) }
    var playing by remember(blobKey) { mutableStateOf(true) }

    LaunchedEffect(blobKey) {
        val file = withContext(Dispatchers.IO) {
            val mp4 = extractEmbeddedMp4(jpegBytes) ?: return@withContext null
            File.createTempFile("secure_motion_", ".mp4", context.cacheDir).apply { writeBytes(mp4) }
        }
        if (file == null) available = false else videoFile = file
    }
    DisposableEffect(videoFile) {
        val f = videoFile
        onDispose { f?.delete() }
    }

    if (!available) return  // no embedded video — the still already shows

    val player = remember(videoFile) {
        videoFile?.let { f ->
            ExoPlayer.Builder(context).build().apply {
                setMediaItem(MediaItem.fromUri(Uri.fromFile(f)))
                repeatMode = Player.REPEAT_MODE_ALL
                volume = 0f
                prepare()
                playWhenReady = true
            }
        }
    }
    DisposableEffect(player) { onDispose { player?.release() } }
    LaunchedEffect(playing, player) { player?.playWhenReady = playing }

    Box(modifier = Modifier.fillMaxSize()) {
        if (player != null && playing) {
            AndroidView(
                modifier = Modifier.fillMaxSize(),
                factory = { ctx -> PlayerView(ctx).apply { useController = false; this.player = player } }
            )
        }
        // LIVE toggle pill (mirrors the main viewer's MotionPhotoOverlay)
        Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
            Surface(
                modifier = Modifier
                    .padding(bottom = 80.dp)
                    .clip(androidx.compose.foundation.shape.CircleShape)
                    .clickable { playing = !playing },
                color = if (playing) Color.White else Color.Black.copy(alpha = 0.6f),
                shape = androidx.compose.foundation.shape.CircleShape
            ) {
                Text(
                    text = if (playing) "LIVE ●" else "LIVE ○",
                    color = if (playing) Color.Black else Color.White,
                    fontWeight = FontWeight.Bold,
                    fontSize = 12.sp,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp)
                )
            }
        }
    }
}

/**
 * Find an embedded MP4 trailer in a motion-photo JPEG by scanning for the
 * `ftyp` box signature (the ISO base-media marker). The MP4 begins 4 bytes
 * before `ftyp` (the box-size prefix). Mirrors the server's ftyp confirmation
 * in `extract_motion_video`. Returns null if no plausible trailer is found.
 */
private fun extractEmbeddedMp4(data: ByteArray): ByteArray? {
    var i = 4
    val end = data.size - 4
    while (i <= end) {
        if (data[i] == 'f'.code.toByte() && data[i + 1] == 't'.code.toByte() &&
            data[i + 2] == 'y'.code.toByte() && data[i + 3] == 'p'.code.toByte()
        ) {
            val start = i - 4
            // Require a real trailer to skip a stray 'ftyp' inside the JPEG data.
            if (start > 0 && data.size - start > 4096) {
                return data.copyOfRange(start, data.size)
            }
        }
        i++
    }
    return null
}

/** Collapse burst stacks in secure items: keep the first frame per burstId. */
private fun collapseSecureBursts(items: List<SecureGalleryItem>): List<SecureGalleryItem> {
    val seen = HashSet<String>()
    return items.filter { item ->
        val bid = item.burstId
        if (bid.isNullOrEmpty()) true else seen.add(bid)
    }
}
