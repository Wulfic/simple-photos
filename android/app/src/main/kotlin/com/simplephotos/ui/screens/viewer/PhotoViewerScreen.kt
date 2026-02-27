package com.simplephotos.ui.screens.viewer

import android.graphics.BitmapFactory
import android.net.Uri
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import androidx.media3.common.MediaItem
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.AddTagRequest
import com.simplephotos.data.remote.dto.RemoveTagRequest
import com.simplephotos.data.repository.PhotoRepository
import com.simplephotos.R
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import javax.inject.Inject

// ---------------------------------------------------------------------------
// ViewModel — loads photo list for paging + handles deletion
// ---------------------------------------------------------------------------

@HiltViewModel
class PhotoViewerViewModel @Inject constructor(
    private val photoRepository: PhotoRepository,
    private val api: ApiService,
    savedStateHandle: SavedStateHandle
) : ViewModel() {

    private val initialPhotoId: String = savedStateHandle["photoId"] ?: ""

    /** Full photo list for paging (matches gallery order). */
    var allPhotos by mutableStateOf<List<PhotoEntity>>(emptyList())
        private set

    /** Index of the photo that was tapped in the gallery. */
    var initialPage by mutableStateOf(0)
        private set

    /** True while the photo list is still loading. */
    var listLoading by mutableStateOf(true)
        private set

    var encryptionMode by mutableStateOf("plain")
        private set

    var serverBaseUrl by mutableStateOf("")
        private set

    var error by mutableStateOf<String?>(null)
        private set

    /** Tags for the currently viewed photo (plain mode only). */
    var currentTags by mutableStateOf<List<String>>(emptyList())
        private set

    /** All user tags for suggestions. */
    var allTags by mutableStateOf<List<String>>(emptyList())
        private set

    /** Favorite state for the currently viewed photo. */
    var isFavorite by mutableStateOf(false)
        private set

    init {
        loadPhotos()
    }

    private fun loadPhotos() {
        viewModelScope.launch {
            try {
                serverBaseUrl = withContext(Dispatchers.IO) { photoRepository.getServerBaseUrl() }
                encryptionMode = withContext(Dispatchers.IO) { photoRepository.getEncryptionMode() }

                val photos = withContext(Dispatchers.IO) {
                    photoRepository.getAllPhotos().first()
                }
                allPhotos = photos
                initialPage = photos.indexOfFirst { it.localId == initialPhotoId }
                    .coerceAtLeast(0)
            } catch (e: Exception) {
                error = e.message
            } finally {
                listLoading = false
            }
        }
    }

    /**
     * Download and decrypt an encrypted blob, returning the raw media bytes.
     * Called from per-page composables for encrypted-mode photos.
     */
    suspend fun downloadAndDecrypt(blobId: String): ByteArray = withContext(Dispatchers.IO) {
        val decrypted = photoRepository.downloadAndDecryptBlob(blobId)
        val payload = JSONObject(String(decrypted, Charsets.UTF_8))
        val dataBase64 = payload.getString("data")
        android.util.Base64.decode(dataBase64, android.util.Base64.NO_WRAP)
    }

    fun deletePhoto(photo: PhotoEntity, onDeleted: () -> Unit) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { photoRepository.deletePhoto(photo) }
                onDeleted()
            } catch (e: Exception) {
                error = e.message
            }
        }
    }

    /** Load tags for a specific photo (called when page changes). */
    fun loadTagsForPhoto(photoId: String?) {
        if (photoId == null || encryptionMode != "plain") return
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) { api.getPhotoTags(photoId) }
                currentTags = response.tags
                // Also refresh all-tags list
                val tagsResponse = withContext(Dispatchers.IO) { api.listTags() }
                allTags = tagsResponse.tags
            } catch (_: Exception) {
                currentTags = emptyList()
            }
        }
    }

    /** Add a tag to the current photo. */
    fun addTag(photoId: String, tag: String) {
        val cleaned = tag.trim().lowercase()
        if (cleaned.isEmpty()) return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { api.addTag(photoId, AddTagRequest(cleaned)) }
                if (!currentTags.contains(cleaned)) {
                    currentTags = (currentTags + cleaned).sorted()
                }
                if (!allTags.contains(cleaned)) {
                    allTags = (allTags + cleaned).sorted()
                }
            } catch (_: Exception) {}
        }
    }

    /** Remove a tag from the current photo. */
    fun removeTag(photoId: String, tag: String) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { api.removeTag(photoId, RemoveTagRequest(tag)) }
                currentTags = currentTags.filter { it != tag }
            } catch (_: Exception) {}
        }
    }

    /** Load favorite state for a specific photo (called when page changes). */
    fun loadFavoriteForPhoto(photoId: String?) {
        if (photoId == null || encryptionMode != "plain") return
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) {
                    api.listPhotos(limit = 500)
                }
                val photo = response.photos.find { it.id == photoId }
                isFavorite = photo?.isFavorite ?: false
            } catch (_: Exception) {
                isFavorite = false
            }
        }
    }

    /** Toggle the favorite state of the current photo. */
    fun toggleFavorite(photoId: String) {
        viewModelScope.launch {
            try {
                val response = withContext(Dispatchers.IO) { api.toggleFavorite(photoId) }
                isFavorite = response.isFavorite
            } catch (_: Exception) {}
        }
    }
}

// ---------------------------------------------------------------------------
// Screen — HorizontalPager for swipe navigation between photos
// ---------------------------------------------------------------------------

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun PhotoViewerScreen(
    onBack: () -> Unit,
    viewModel: PhotoViewerViewModel = hiltViewModel()
) {
    // Wait for the photo list before creating pager state
    if (viewModel.listLoading) {
        Box(
            modifier = Modifier.fillMaxSize().background(Color.Black),
            contentAlignment = Alignment.Center
        ) {
            CircularProgressIndicator(color = Color.White)
        }
        return
    }

    if (viewModel.allPhotos.isEmpty()) {
        Box(
            modifier = Modifier.fillMaxSize().background(Color.Black),
            contentAlignment = Alignment.Center
        ) {
            Text("Photo not found", color = Color.White)
        }
        return
    }

    val pagerState = rememberPagerState(
        initialPage = viewModel.initialPage,
        pageCount = { viewModel.allPhotos.size }
    )

    val currentPhoto = viewModel.allPhotos.getOrNull(pagerState.currentPage)

    // Load tags and favorite when page changes (plain mode only)
    val isPlainMode = viewModel.encryptionMode == "plain"
    LaunchedEffect(pagerState.currentPage) {
        val photo = viewModel.allPhotos.getOrNull(pagerState.currentPage)
        if (isPlainMode && photo?.serverPhotoId != null) {
            viewModel.loadTagsForPhoto(photo.serverPhotoId)
            viewModel.loadFavoriteForPhoto(photo.serverPhotoId)
        }
    }

    // Tag input state
    var showTagInput by remember { mutableStateOf(false) }
    var tagInputText by remember { mutableStateOf("") }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(
                            text = currentPhoto?.filename ?: "Viewer",
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                        Text(
                            text = "${pagerState.currentPage + 1} / ${viewModel.allPhotos.size}",
                            style = MaterialTheme.typography.bodySmall.copy(fontSize = 12.sp),
                            color = Color.White.copy(alpha = 0.7f)
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(painter = painterResource(R.drawable.ic_back_arrow), contentDescription = "Back")
                    }
                },
                actions = {
                    if (currentPhoto != null) {
                        // Favorite button — plain mode only
                        if (isPlainMode && currentPhoto.serverPhotoId != null) {
                            IconButton(onClick = { viewModel.toggleFavorite(currentPhoto.serverPhotoId!!) }) {
                                Icon(
                                    painter = painterResource(R.drawable.ic_star),
                                    contentDescription = if (viewModel.isFavorite) "Unfavorite" else "Favorite",
                                    tint = if (viewModel.isFavorite) Color(0xFFFBBF24) else Color.White
                                )
                            }
                        }
                        IconButton(onClick = { viewModel.deletePhoto(currentPhoto, onBack) }) {
                            Icon(painter = painterResource(R.drawable.ic_trashcan), contentDescription = "Delete")
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = Color.Black.copy(alpha = 0.7f),
                    titleContentColor = Color.White,
                    navigationIconContentColor = Color.White,
                    actionIconContentColor = Color.White
                )
            )
        },
        containerColor = Color.Black
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
        ) {
            HorizontalPager(
                state = pagerState,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth(),
                key = { viewModel.allPhotos[it].localId }
            ) { page ->
                val photo = viewModel.allPhotos[page]
                PhotoPageContent(
                    photo = photo,
                    encryptionMode = viewModel.encryptionMode,
                    serverBaseUrl = viewModel.serverBaseUrl,
                    viewModel = viewModel
                )
            }

            // Tag bar — plain mode only
            if (isPlainMode && currentPhoto?.serverPhotoId != null) {
                Surface(
                    color = Color.Black.copy(alpha = 0.6f)
                ) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 12.dp, vertical = 8.dp)
                    ) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(6.dp)
                        ) {
                            // Existing tags
                            viewModel.currentTags.forEach { tag ->
                                Surface(
                                    shape = androidx.compose.foundation.shape.RoundedCornerShape(12.dp),
                                    color = Color(0xFF1E40AF).copy(alpha = 0.4f)
                                ) {
                                    Row(
                                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                                        verticalAlignment = Alignment.CenterVertically
                                    ) {
                                        Text(tag, color = Color(0xFF93C5FD), fontSize = 12.sp)
                                        Spacer(Modifier.width(4.dp))
                                        Text(
                                            "✕",
                                            color = Color(0xFF93C5FD),
                                            fontSize = 10.sp,
                                            modifier = Modifier.clickable {
                                                viewModel.removeTag(currentPhoto.serverPhotoId!!, tag)
                                            }
                                        )
                                    }
                                }
                            }

                            // Add tag button or inline input
                            if (showTagInput) {
                                OutlinedTextField(
                                    value = tagInputText,
                                    onValueChange = { tagInputText = it },
                                    modifier = Modifier
                                        .width(120.dp)
                                        .height(36.dp),
                                    placeholder = { Text("tag", fontSize = 12.sp, color = Color.Gray) },
                                    singleLine = true,
                                    textStyle = LocalTextStyle.current.copy(fontSize = 12.sp, color = Color.White),
                                    colors = OutlinedTextFieldDefaults.colors(
                                        focusedBorderColor = Color(0xFF3B82F6),
                                        unfocusedBorderColor = Color.Gray,
                                        cursorColor = Color.White
                                    )
                                )
                                TextButton(onClick = {
                                    if (tagInputText.isNotBlank()) {
                                        viewModel.addTag(currentPhoto.serverPhotoId!!, tagInputText)
                                        tagInputText = ""
                                    }
                                }) {
                                    Text("Add", color = Color(0xFF60A5FA), fontSize = 12.sp)
                                }
                                TextButton(onClick = { showTagInput = false; tagInputText = "" }) {
                                    Text("✕", color = Color.Gray, fontSize = 12.sp)
                                }
                            } else {
                                Surface(
                                    modifier = Modifier.clickable { showTagInput = true },
                                    shape = androidx.compose.foundation.shape.RoundedCornerShape(12.dp),
                                    color = Color.Transparent,
                                    border = androidx.compose.foundation.BorderStroke(1.dp, Color.Gray.copy(alpha = 0.5f))
                                ) {
                                    Text(
                                        "+ Tag",
                                        color = Color.Gray,
                                        fontSize = 12.sp,
                                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp)
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

// ---------------------------------------------------------------------------
// Per-page content — each page independently loads and renders its photo
// ---------------------------------------------------------------------------

@Composable
private fun PhotoPageContent(
    photo: PhotoEntity,
    encryptionMode: String,
    serverBaseUrl: String,
    viewModel: PhotoViewerViewModel
) {
    val context = LocalContext.current

    // Determine the content source for this photo
    val isPlainMode = encryptionMode == "plain" && photo.serverPhotoId != null
    val hasLocalPath = photo.localPath != null
    val hasEncryptedBlob = photo.serverBlobId != null

    // For encrypted mode, lazily download & decrypt
    var decryptedData by remember(photo.localId) { mutableStateOf<ByteArray?>(null) }
    var decryptLoading by remember(photo.localId) { mutableStateOf(hasEncryptedBlob && !isPlainMode && !hasLocalPath) }
    var decryptError by remember(photo.localId) { mutableStateOf<String?>(null) }

    LaunchedEffect(photo.localId) {
        if (!isPlainMode && !hasLocalPath && hasEncryptedBlob) {
            decryptLoading = true
            try {
                decryptedData = viewModel.downloadAndDecrypt(photo.serverBlobId!!)
            } catch (e: Exception) {
                decryptError = e.message
            } finally {
                decryptLoading = false
            }
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black),
        contentAlignment = Alignment.Center
    ) {
        when {
            // Loading encrypted content
            decryptLoading -> {
                CircularProgressIndicator(color = Color.White)
            }

            decryptError != null -> {
                Text(decryptError ?: "Error", color = Color.White, modifier = Modifier.padding(16.dp))
            }

            // ── Video ──────────────────────────────────────────────────
            photo.mediaType == "video" -> {
                val videoUri = when {
                    isPlainMode -> Uri.parse("$serverBaseUrl/api/photos/${photo.serverPhotoId}/file")
                    hasLocalPath -> Uri.parse(photo.localPath)
                    decryptedData != null -> {
                        val tempFile = remember(photo.localId, decryptedData) {
                            java.io.File.createTempFile("video_", ".mp4", context.cacheDir).apply {
                                writeBytes(decryptedData!!)
                                deleteOnExit()
                            }
                        }
                        Uri.fromFile(tempFile)
                    }
                    else -> null
                }
                if (videoUri != null) {
                    VideoPlayer(uri = videoUri)
                } else {
                    Text("Video not available", color = Color.White)
                }
            }

            // ── Photo / GIF ────────────────────────────────────────────
            else -> {
                when {
                    // Plain mode: Coil loads via authenticated URL
                    isPlainMode -> {
                        AsyncImage(
                            model = ImageRequest.Builder(context)
                                .data("$serverBaseUrl/api/photos/${photo.serverPhotoId}/file")
                                .crossfade(true)
                                .build(),
                            contentDescription = photo.filename,
                            modifier = Modifier.fillMaxSize(),
                            contentScale = ContentScale.Fit
                        )
                    }
                    // Local file
                    hasLocalPath -> {
                        val bitmap = remember(photo.localPath) {
                            try {
                                val stream = context.contentResolver.openInputStream(Uri.parse(photo.localPath))
                                BitmapFactory.decodeStream(stream)
                            } catch (_: Exception) { null }
                        }
                        bitmap?.let {
                            Image(
                                bitmap = it.asImageBitmap(),
                                contentDescription = photo.filename,
                                modifier = Modifier.fillMaxSize(),
                                contentScale = ContentScale.Fit
                            )
                        }
                    }
                    // Decrypted blob
                    decryptedData != null -> {
                        val bitmap = remember(decryptedData) {
                            BitmapFactory.decodeByteArray(decryptedData, 0, decryptedData!!.size)
                        }
                        bitmap?.let {
                            Image(
                                bitmap = it.asImageBitmap(),
                                contentDescription = photo.filename,
                                modifier = Modifier.fillMaxSize(),
                                contentScale = ContentScale.Fit
                            )
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Video player composable (ExoPlayer)
// ---------------------------------------------------------------------------

@Composable
private fun VideoPlayer(uri: Uri) {
    val context = LocalContext.current
    val player = remember {
        ExoPlayer.Builder(context).build().apply {
            setMediaItem(MediaItem.fromUri(uri))
            prepare()
        }
    }

    DisposableEffect(Unit) {
        onDispose { player.release() }
    }

    AndroidView(
        factory = {
            PlayerView(it).apply {
                this.player = player
                useController = true
            }
        },
        modifier = Modifier.fillMaxSize()
    )
}
