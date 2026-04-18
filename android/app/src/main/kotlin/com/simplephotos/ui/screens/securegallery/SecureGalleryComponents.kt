@file:OptIn(ExperimentalMaterial3Api::class)

package com.simplephotos.ui.screens.securegallery

import android.graphics.BitmapFactory
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
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
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem

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
    onDeleteGallery: (SecureGallery) -> Unit
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
                                Surface(
                                    modifier = Modifier.size(48.dp),
                                    shape = RoundedCornerShape(8.dp),
                                    color = MaterialTheme.colorScheme.primaryContainer
                                ) {
                                    Box(contentAlignment = Alignment.Center) {
                                        Icon(
                                            painter = painterResource(R.drawable.ic_locks),
                                            contentDescription = null,
                                            tint = MaterialTheme.colorScheme.primary
                                        )
                                    }
                                }
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
    val availablePhotos = remember(allPhotos, albumBlobIds) {
        allPhotos.filter { it.serverBlobId != null && it.serverBlobId !in albumBlobIds }
    }

    // Full-screen viewer for secure items only
    if (viewerIndex != null) {
        SecurePhotoViewer(
            items = items,
            initialIndex = viewerIndex!!,
            viewModel = viewModel,
            onBack = { viewerIndex = null }
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
                            "${items.size} items",
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
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(80.dp),
                        contentPadding = PaddingValues(4.dp),
                        horizontalArrangement = Arrangement.spacedBy(2.dp),
                        verticalArrangement = Arrangement.spacedBy(2.dp),
                        modifier = Modifier.weight(1f)
                    ) {
                        items(availablePhotos, key = { it.localId }) { photo ->
                            val blobId = photo.serverBlobId ?: return@items
                            val isSelected = blobId in selectedBlobIds
                            Box(
                                modifier = Modifier
                                    .aspectRatio(1f)
                                    .clip(RoundedCornerShape(4.dp))
                                    .clickable {
                                        selectedBlobIds = if (isSelected)
                                            selectedBlobIds - blobId
                                        else
                                            selectedBlobIds + blobId
                                    }
                            ) {
                                PhotoThumbnail(photo)
                                if (isSelected) {
                                    Box(
                                        modifier = Modifier
                                            .fillMaxSize()
                                            .background(Color(0xFF3B82F6).copy(alpha = 0.3f))
                                    )
                                    Surface(
                                        modifier = Modifier
                                            .align(Alignment.TopEnd)
                                            .padding(4.dp)
                                            .size(20.dp),
                                        shape = androidx.compose.foundation.shape.CircleShape,
                                        color = Color(0xFF3B82F6)
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
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(100.dp),
                        contentPadding = PaddingValues(2.dp),
                        horizontalArrangement = Arrangement.spacedBy(2.dp),
                        verticalArrangement = Arrangement.spacedBy(2.dp)
                    ) {
                        items(items, key = { it.id }) { item ->
                            SecureItemTile(
                                item = item,
                                onClick = {
                                    val idx = items.indexOfFirst { it.blobId == item.blobId }
                                    viewerIndex = idx.coerceAtLeast(0)
                                },
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
// Secure Item Tile — downloads and shows decrypted thumbnail
// ─────────────────────────────────────────────────────────────────────────────

@Composable
internal fun SecureItemTile(
    item: SecureGalleryItem,
    onClick: () -> Unit,
    viewModel: SecureGalleryViewModel
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
    onBack: () -> Unit
) {
    val pagerState = rememberPagerState(
        initialPage = initialIndex.coerceIn(0, (items.size - 1).coerceAtLeast(0)),
        pageCount = { items.size }
    )

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black)
    ) {
        HorizontalPager(
            state = pagerState,
            modifier = Modifier.fillMaxSize()
        ) { page ->
            val item = items[page]
            var bitmap by remember(item.blobId) { mutableStateOf<android.graphics.Bitmap?>(null) }
            var gifBytes by remember(item.blobId) { mutableStateOf<ByteArray?>(null) }
            var loading by remember(item.blobId) { mutableStateOf(true) }

            LaunchedEffect(item.blobId) {
                loading = true
                try {
                    val data = viewModel.downloadAndDecrypt(item.blobId)
                    val isGif = data.size > 3 &&
                        data[0] == 0x47.toByte() && data[1] == 0x49.toByte() && data[2] == 0x46.toByte()
                    if (isGif) {
                        gifBytes = data
                    } else {
                        bitmap = BitmapFactory.decodeByteArray(data, 0, data.size)
                    }
                } catch (e: Exception) {
                    android.util.Log.e("SecurePhotoViewer", "Failed to decrypt blobId=${item.blobId}", e)
                    bitmap = null
                    gifBytes = null
                } finally {
                    loading = false
                }
            }

            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                when {
                    loading -> CircularProgressIndicator(color = Color.White)
                    gifBytes != null -> AsyncImage(
                        model = ImageRequest.Builder(LocalContext.current)
                            .data(java.nio.ByteBuffer.wrap(gifBytes!!))
                            .build(),
                        contentDescription = "Secure photo",
                        modifier = Modifier.fillMaxSize(),
                        contentScale = ContentScale.Fit
                    )
                    bitmap != null -> Image(
                        bitmap = bitmap!!.asImageBitmap(),
                        contentDescription = "Secure photo",
                        modifier = Modifier.fillMaxSize(),
                        contentScale = ContentScale.Fit
                    )
                    else -> Text("Failed to decrypt", color = Color.White)
                }
            }
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
    }
}
