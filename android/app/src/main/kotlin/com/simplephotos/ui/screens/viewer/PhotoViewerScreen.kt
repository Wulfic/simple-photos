package com.simplephotos.ui.screens.viewer

import android.app.Activity
import android.content.ContentValues
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.cache.CacheDataSource
import androidx.media3.datasource.cache.LeastRecentlyUsedCacheEvictor
import androidx.media3.datasource.cache.SimpleCache
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import java.io.File
import coil.imageLoader
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.R
import kotlinx.coroutines.launch
import org.json.JSONObject

// Mutable crop region used during edit mode (normalised 0..1)
private data class CropCorners(
    val x: Float,
    val y: Float,
    val w: Float,
    val h: Float
)

// ---------------------------------------------------------------------------
// Screen — HorizontalPager for swipe navigation between photos
// ---------------------------------------------------------------------------

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@androidx.annotation.OptIn(UnstableApi::class)
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
    val context = LocalContext.current

    // ── Disk-based media cache ────────────────────────────────────────
    // ExoPlayer's in-memory buffer lives in the Java heap (capped at
    // ~512 MB by Android).  Instead of trying to fit large videos in
    // that heap, we use a 512 MB **disk** cache (via SimpleCache +
    // CacheDataSource).  The heap now only holds a tiny 2–5 MB decode
    // window while the disk cache absorbs the full stream — exactly
    // how YouTube / Netflix handle 4K and 8K.
    // This lets us effectively use GBs of storage as RAM, bypassing
    // the Java heap limit entirely for buffered video data.
    val diskCache = remember {
        val cacheDir = File(context.cacheDir, "exo_media_cache")
        if (!cacheDir.exists()) cacheDir.mkdirs()
        // 512 MB disk cache — evicts least-recently-used segments.
        // Storage is plentiful; this is the "more RAM" the user is
        // looking for, just stored on flash instead of DRAM.
        val evictor = LeastRecentlyUsedCacheEvictor(512L * 1024 * 1024)
        SimpleCache(cacheDir, evictor)
    }

    // ── Single shared ExoPlayer ──────────────────────────────────────
    // One ExoPlayer lives for the entire viewer screen. When the user
    // swipes to a different video, we swap the MediaItem instead of
    // creating a second player. This guarantees only ONE MediaCodec
    // instance ever exists, preventing the 128 MB frame-buffer OOM.
    val sharedPlayer = remember {
        run {
            val httpFactory = OkHttpDataSource.Factory(viewModel.okHttpClient)
            val upstreamFactory = DefaultDataSource.Factory(context, httpFactory)

            // Wrap the network data source with a disk cache.
            // Reads: cache-hit → serve from disk; cache-miss → fetch
            // from network, write-through to disk, then serve.
            // This means the Java heap never holds the full video.
            val cacheDataSourceFactory = CacheDataSource.Factory()
                .setCache(diskCache)
                .setUpstreamDataSourceFactory(upstreamFactory)
                .setFlags(CacheDataSource.FLAG_IGNORE_CACHE_ON_ERROR)

            val mediaSourceFactory = DefaultMediaSourceFactory(cacheDataSourceFactory)

            // ── Tiny in-memory buffer ────────────────────────────────
            // Since the disk cache handles buffering, the in-memory
            // buffer only needs to hold a few seconds of decode-ready
            // data.  We keep it intentionally small (2.5 MB) so the
            // Java heap has maximum room for the codec output buffers,
            // Coil, and Compose.
            val heapBytes = Runtime.getRuntime().maxMemory()
            Log.d("VideoPlayer", "Heap=${heapBytes/1024/1024}MB — using disk cache (512 MB), heap buffer=2.5 MB")

            val loadControl = DefaultLoadControl.Builder()
                .setBufferDurationsMs(
                    /* minBufferMs */        2_500,   // 2.5 s — just enough to start
                    /* maxBufferMs */       30_000,   // 30 s — the disk backs this up
                    /* bufferForPlaybackMs */  1_000,
                    /* bufferForPlaybackAfterRebufferMs */ 2_000
                )
                // 2.5 MB heap-resident buffer — the rest lives on disk.
                // Even 4K/70 Mbps only uses ~9 MB/s ≈ 0.3 s in 2.5 MB.
                // The player will briefly re-read from disk cache, which
                // is fast enough (100+ MB/s on modern flash).
                .setTargetBufferBytes(2_500_000)
                .setPrioritizeTimeOverSizeThresholds(false)
                .build()

            // ── Renderer config ──────────────────────────────────────
            val renderersFactory = DefaultRenderersFactory(context)
                .setEnableDecoderFallback(true)
                .setExtensionRendererMode(
                    DefaultRenderersFactory.EXTENSION_RENDERER_MODE_PREFER
                )

            ExoPlayer.Builder(context)
                .setMediaSourceFactory(mediaSourceFactory)
                .setLoadControl(loadControl)
                .setRenderersFactory(renderersFactory)
                .build()
                .apply {
                    playWhenReady = false
                    videoChangeFrameRateStrategy =
                        C.VIDEO_CHANGE_FRAME_RATE_STRATEGY_OFF
                }
        }
    }
    // Release the shared player AND disk cache when the viewer leaves
    DisposableEffect(Unit) {
        onDispose {
            sharedPlayer.stop()
            sharedPlayer.release()
            diskCache.release()
        }
    }
    // Track the URI currently loaded into the shared player so we only
    // call setMediaItem when it actually changes.
    var activeVideoUri by remember { mutableStateOf<Uri?>(null) }
    // Track player errors at this level so the page can display them
    var sharedPlayerError by remember { mutableStateOf<String?>(null) }
    var sharedPlayerConverting by remember { mutableStateOf(false) }
    DisposableEffect(sharedPlayer) {
        val listener = object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                Log.w("VideoPlayer", "ExoPlayer error: ${error.message}")
                sharedPlayerError = error.message ?: "Cannot play this video"
            }
        }
        sharedPlayer.addListener(listener)
        onDispose { sharedPlayer.removeListener(listener) }
    }

    // Stop the player when the user swipes from a video to a photo page.
    // Without this, audio continues playing in the background.
    LaunchedEffect(pagerState.currentPage) {
        val photo = viewModel.allPhotos.getOrNull(pagerState.currentPage)
        val isVideo = photo?.mediaType == "video" || photo?.mediaType == "audio"
        if (!isVideo) {
            sharedPlayer.playWhenReady = false
            sharedPlayer.stop()
            activeVideoUri = null
            sharedPlayerError = null
        }
    }

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

    // Controls overlay visibility — tap photo to toggle
    var showOverlay by remember { mutableStateOf(true) }

    // ── Info panel state ─────────────────────────────────────────────
    var showInfoPanel by remember { mutableStateOf(false) }

    // ── Download state ───────────────────────────────────────────────
    val scope = rememberCoroutineScope()
    var downloadMessage by remember { mutableStateOf<String?>(null) }

    // ── Edit mode state ──────────────────────────────────────────────
    var editMode by remember { mutableStateOf(false) }
    var editTab by remember { mutableStateOf("brightness") } // "crop" | "brightness"
    var cropCorners by remember { mutableStateOf(CropCorners(0f, 0f, 1f, 1f)) }
    var brightnessValue by remember { mutableStateOf(0f) }

    // Initialize edit mode from existing crop metadata
    fun enterEditMode() {
        val photo = currentPhoto ?: return
        val cm = photo.cropMetadata
        if (cm != null) {
            try {
                val json = JSONObject(cm)
                cropCorners = CropCorners(
                    x = json.optDouble("x", 0.0).toFloat(),
                    y = json.optDouble("y", 0.0).toFloat(),
                    w = json.optDouble("width", 1.0).toFloat(),
                    h = json.optDouble("height", 1.0).toFloat()
                )
                brightnessValue = json.optDouble("brightness", 0.0).toFloat()
            } catch (_: Exception) {
                cropCorners = CropCorners(0f, 0f, 1f, 1f)
                brightnessValue = 0f
            }
        } else {
            cropCorners = CropCorners(0f, 0f, 1f, 1f)
            brightnessValue = 0f
        }
        editTab = "brightness"
        editMode = true
    }

    fun saveEdit() {
        val photo = currentPhoto ?: return
        val c = cropCorners
        val isDefaultCrop = c.x <= 0.01f && c.y <= 0.01f && c.w >= 0.99f && c.h >= 0.99f
        val isDefaultBrightness = kotlin.math.abs(brightnessValue) < 1f

        if (isDefaultCrop && isDefaultBrightness) {
            viewModel.saveCropMetadata(photo, null)
        } else {
            val meta = JSONObject().apply {
                put("x", c.x.toDouble().coerceIn(0.0, 1.0))
                put("y", c.y.toDouble().coerceIn(0.0, 1.0))
                put("width", c.w.toDouble().coerceIn(0.05, 1.0))
                put("height", c.h.toDouble().coerceIn(0.05, 1.0))
                put("rotate", 0)
                put("brightness", brightnessValue.toDouble())
            }.toString()
            viewModel.saveCropMetadata(photo, meta)
        }
        editMode = false
    }

    fun clearCrop() {
        val photo = currentPhoto ?: return
        viewModel.saveCropMetadata(photo, null)
        cropCorners = CropCorners(0f, 0f, 1f, 1f)
        brightnessValue = 0f
    }

    // Immersive full-screen: hide system bars when viewer is open
    val view = LocalView.current
    DisposableEffect(Unit) {
        val activity = view.context as? Activity ?: return@DisposableEffect onDispose {}
        val window = activity.window
        val controller = WindowCompat.getInsetsController(window, view)
        controller.hide(WindowInsetsCompat.Type.systemBars())
        controller.systemBarsBehavior =
            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        onDispose {
            controller.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    // Vertical swipe threshold for gestures (dp → px converted at composition time)
    val verticalSwipeThreshold = 100f

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black)
            .pointerInput(editMode) {
                // Detect vertical swipes: up → info panel, down → close viewer
                // Only at the top level so it doesn't conflict with zoom panning
                if (editMode) return@pointerInput
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    var totalY = 0f
                    var totalX = 0f
                    var consumed = false
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull() ?: break
                        if (!change.pressed) {
                            // Finger lifted — evaluate the swipe
                            if (!consumed && kotlin.math.abs(totalY) > verticalSwipeThreshold &&
                                kotlin.math.abs(totalY) > kotlin.math.abs(totalX) * 1.5f
                            ) {
                                if (totalY < 0) {
                                    // Swipe up → show info panel
                                    showInfoPanel = true
                                } else {
                                    // Swipe down → close viewer
                                    if (showInfoPanel) {
                                        showInfoPanel = false
                                    } else {
                                        onBack()
                                    }
                                }
                            }
                            break
                        }
                        // Only track single-finger vertical movement
                        if (event.changes.count { it.pressed } == 1) {
                            totalY += change.positionChange().y
                            totalX += change.positionChange().x
                        } else {
                            consumed = true // multi-touch — don't interpret as swipe
                        }
                    }
                }
            }
    ) {
        // ── Full-screen pager (behind overlays) ────────────────────────
        HorizontalPager(
            state = pagerState,
            modifier = Modifier.fillMaxSize(),
            beyondBoundsPageCount = 0,
            userScrollEnabled = !editMode,
            key = { viewModel.allPhotos.getOrNull(it)?.localId ?: it }
        ) { page ->
            val photo = viewModel.allPhotos.getOrNull(page)
            if (photo == null) {
                Box(
                    modifier = Modifier.fillMaxSize().background(Color.Black),
                    contentAlignment = Alignment.Center
                ) {
                    Text("Photo not available", color = Color.White)
                }
                return@HorizontalPager
            }
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .clickable(
                        indication = null,
                        interactionSource = remember { androidx.compose.foundation.interaction.MutableInteractionSource() }
                    ) { showOverlay = !showOverlay }
            ) {
                PhotoPageContent(
                    photo = photo,
                    encryptionMode = viewModel.encryptionMode,
                    serverBaseUrl = viewModel.serverBaseUrl,
                    viewModel = viewModel,
                    okHttpClient = viewModel.okHttpClient,
                    isActivePage = pagerState.currentPage == page,
                    editMode = editMode,
                    editBrightness = brightnessValue,
                    sharedPlayer = sharedPlayer,
                    activeVideoUri = activeVideoUri,
                    onVideoUriReady = { uri, filename ->
                        // Load the new media item into the shared player.
                        // Only swap if the URI actually changed (avoids re-prepare
                        // on every recomposition).
                        if (uri != activeVideoUri) {
                            sharedPlayerError = null
                            sharedPlayerConverting = needsWebPreview(filename) != null
                            activeVideoUri = uri

                            // Free Coil's in-memory bitmap cache — the cached
                            // photo bitmaps are invisible while a video plays
                            // and just waste heap that the codec needs.
                            context.imageLoader.memoryCache?.clear()

                            // Hint the GC to reclaim soft references / finalizer
                            // -queued bitmaps before the codec allocates its
                            // output buffers.
                            @Suppress("ExplicitGarbageCollection")
                            System.gc()

                            // Don't call stop() first — it releases the MediaCodec,
                            // then prepare() allocates a new one.  The old native
                            // buffers (~128 MB for 4K) may not be freed yet,
                            // causing a transient peak that triggers OOM.
                            // setMediaItem + prepare reuses the codec if the format
                            // is compatible (e.g., both H.264 MP4).
                            sharedPlayer.setMediaItem(MediaItem.fromUri(uri))
                            sharedPlayer.prepare()
                            sharedPlayer.playWhenReady = true
                        }
                    },
                    playerError = sharedPlayerError,
                    isConverting = sharedPlayerConverting
                )
            }
        }

        // ── Crop overlay drawn on top of the full-screen photo ─────────
        if (editMode && editTab == "crop") {
            BoxWithConstraints(
                modifier = Modifier.fillMaxSize()
            ) {
                val bw = constraints.maxWidth.toFloat()
                val bh = constraints.maxHeight.toFloat()
                val cc = cropCorners
                val handleRadius = 10.dp

                // Darkened overlay (semi-transparent) outside crop region
                Canvas(modifier = Modifier.fillMaxSize()) {
                    val left = cc.x * size.width
                    val top = cc.y * size.height
                    val right = (cc.x + cc.w) * size.width
                    val bottom = (cc.y + cc.h) * size.height

                    val dimColor = Color.Black.copy(alpha = 0.5f)
                    // Top
                    drawRect(dimColor, topLeft = Offset.Zero, size = androidx.compose.ui.geometry.Size(size.width, top))
                    // Bottom
                    drawRect(dimColor, topLeft = Offset(0f, bottom), size = androidx.compose.ui.geometry.Size(size.width, size.height - bottom))
                    // Left
                    drawRect(dimColor, topLeft = Offset(0f, top), size = androidx.compose.ui.geometry.Size(left, bottom - top))
                    // Right
                    drawRect(dimColor, topLeft = Offset(right, top), size = androidx.compose.ui.geometry.Size(size.width - right, bottom - top))

                    // Crop border
                    drawRect(
                        Color.White,
                        topLeft = Offset(left, top),
                        size = androidx.compose.ui.geometry.Size(right - left, bottom - top),
                        style = Stroke(width = 2f)
                    )
                }

                // Corner handles
                val corners = listOf(
                    "tl" to Offset(cc.x * bw, cc.y * bh),
                    "tr" to Offset((cc.x + cc.w) * bw, cc.y * bh),
                    "bl" to Offset(cc.x * bw, (cc.y + cc.h) * bh),
                    "br" to Offset((cc.x + cc.w) * bw, (cc.y + cc.h) * bh)
                )

                for ((corner, pos) in corners) {
                    Box(
                        modifier = Modifier
                            .offset(
                                x = with(androidx.compose.ui.platform.LocalDensity.current) { (pos.x).toDp() } - handleRadius,
                                y = with(androidx.compose.ui.platform.LocalDensity.current) { (pos.y).toDp() } - handleRadius
                            )
                            .size(handleRadius * 2)
                            .clip(CircleShape)
                            .background(Color.White)
                            .pointerInput(corner) {
                                detectDragGestures { change, dragAmount ->
                                    change.consume()
                                    val minSize = 0.05f
                                    val dx = dragAmount.x / bw
                                    val dy = dragAmount.y / bh
                                    cropCorners = when (corner) {
                                        "tl" -> {
                                            val newX = (cropCorners.x + dx).coerceIn(0f, cropCorners.x + cropCorners.w - minSize)
                                            val newY = (cropCorners.y + dy).coerceIn(0f, cropCorners.y + cropCorners.h - minSize)
                                            CropCorners(
                                                x = newX,
                                                y = newY,
                                                w = cropCorners.w + (cropCorners.x - newX),
                                                h = cropCorners.h + (cropCorners.y - newY)
                                            )
                                        }
                                        "tr" -> {
                                            val newR = (cropCorners.x + cropCorners.w + dx).coerceIn(cropCorners.x + minSize, 1f)
                                            val newY = (cropCorners.y + dy).coerceIn(0f, cropCorners.y + cropCorners.h - minSize)
                                            CropCorners(
                                                x = cropCorners.x,
                                                y = newY,
                                                w = newR - cropCorners.x,
                                                h = cropCorners.h + (cropCorners.y - newY)
                                            )
                                        }
                                        "bl" -> {
                                            val newX = (cropCorners.x + dx).coerceIn(0f, cropCorners.x + cropCorners.w - minSize)
                                            val newB = (cropCorners.y + cropCorners.h + dy).coerceIn(cropCorners.y + minSize, 1f)
                                            CropCorners(
                                                x = newX,
                                                y = cropCorners.y,
                                                w = cropCorners.w + (cropCorners.x - newX),
                                                h = newB - cropCorners.y
                                            )
                                        }
                                        "br" -> {
                                            val newR = (cropCorners.x + cropCorners.w + dx).coerceIn(cropCorners.x + minSize, 1f)
                                            val newB = (cropCorners.y + cropCorners.h + dy).coerceIn(cropCorners.y + minSize, 1f)
                                            CropCorners(
                                                x = cropCorners.x,
                                                y = cropCorners.y,
                                                w = newR - cropCorners.x,
                                                h = newB - cropCorners.y
                                            )
                                        }
                                        else -> cropCorners
                                    }
                                }
                            }
                    )
                }
            }
        }

        // ── Top bar overlay ────────────────────────────────────────────
        androidx.compose.animation.AnimatedVisibility(
            visible = showOverlay,
            enter = androidx.compose.animation.fadeIn(),
            exit = androidx.compose.animation.fadeOut(),
            modifier = Modifier.align(Alignment.TopCenter)
        ) {
            Surface(
                color = Color.Black.copy(alpha = 0.7f),
                modifier = Modifier.fillMaxWidth()
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .windowInsetsPadding(WindowInsets.statusBars)
                        .padding(top = 12.dp) // extra padding below notch / camera cutout
                        .padding(horizontal = 4.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    IconButton(onClick = onBack, modifier = Modifier.size(32.dp)) {
                        Icon(painter = painterResource(R.drawable.ic_back_arrow), contentDescription = "Back", tint = Color.White, modifier = Modifier.size(12.dp))
                    }
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = currentPhoto?.filename ?: "Viewer",
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            color = Color.White,
                            style = MaterialTheme.typography.titleSmall
                        )
                        Text(
                            text = "${pagerState.currentPage + 1} / ${viewModel.allPhotos.size}",
                            style = MaterialTheme.typography.bodySmall.copy(fontSize = 12.sp),
                            color = Color.White.copy(alpha = 0.7f)
                        )
                    }
                    if (currentPhoto != null) {
                        if (isPlainMode && currentPhoto.serverPhotoId != null) {
                            IconButton(onClick = { viewModel.toggleFavorite(currentPhoto.serverPhotoId!!) }) {
                                Icon(
                                    painter = painterResource(R.drawable.ic_star),
                                    contentDescription = if (viewModel.isFavorite) "Unfavorite" else "Favorite",
                                    tint = if (viewModel.isFavorite) Color(0xFFFBBF24) else Color.White,
                                    modifier = Modifier.size(12.dp)
                                )
                            }
                        }
                        // Info button
                        IconButton(onClick = { showInfoPanel = !showInfoPanel }) {
                            Icon(
                                imageVector = Icons.Default.Info,
                                contentDescription = "Info",
                                tint = if (showInfoPanel) Color(0xFF60A5FA) else Color.White,
                                modifier = Modifier.size(18.dp)
                            )
                        }
                        // Edit button — only for photos (not videos/gifs)
                        if (currentPhoto.mediaType != "video") {
                            TextButton(onClick = {
                                if (editMode) {
                                    editMode = false
                                } else {
                                    enterEditMode()
                                }
                            }) {
                                Text(
                                    if (editMode) "Done" else "Edit",
                                    color = if (editMode) Color(0xFF60A5FA) else Color.White,
                                    fontSize = 14.sp
                                )
                            }
                        }
                        // Download button
                        IconButton(onClick = {
                            val photo = currentPhoto ?: return@IconButton
                            scope.launch {
                                try {
                                    // For local files, use content resolver; for server files, download
                                    val bytes: ByteArray? = when {
                                        photo.localPath != null -> {
                                            try {
                                                context.contentResolver.openInputStream(Uri.parse(photo.localPath))?.readBytes()
                                            } catch (_: Exception) { null }
                                        }
                                        else -> viewModel.downloadPhotoBytes(photo)
                                    }
                                    if (bytes != null) {
                                        val values = ContentValues().apply {
                                            put(MediaStore.MediaColumns.DISPLAY_NAME, photo.filename)
                                            put(MediaStore.MediaColumns.MIME_TYPE, when {
                                                photo.filename.endsWith(".png", true) -> "image/png"
                                                photo.filename.endsWith(".gif", true) -> "image/gif"
                                                photo.filename.endsWith(".mp4", true) -> "video/mp4"
                                                photo.filename.endsWith(".webm", true) -> "video/webm"
                                                else -> "image/jpeg"
                                            })
                                            put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
                                        }
                                        val uri = context.contentResolver.insert(
                                            MediaStore.Downloads.EXTERNAL_CONTENT_URI, values
                                        )
                                        uri?.let { context.contentResolver.openOutputStream(it)?.use { os -> os.write(bytes) } }
                                        downloadMessage = "Saved to Downloads"
                                    } else {
                                        downloadMessage = "Download failed"
                                    }
                                } catch (e: Exception) {
                                    downloadMessage = "Download failed: ${e.message}"
                                }
                            }
                        }) {
                            Icon(
                                painter = painterResource(R.drawable.ic_download),
                                contentDescription = "Download",
                                tint = Color.White,
                                modifier = Modifier.size(12.dp)
                            )
                        }
                        if (viewModel.albumId != null) {
                            // Album context: remove from album only (don't delete the photo)
                            IconButton(onClick = { viewModel.removeFromAlbum(currentPhoto, onBack) }) {
                                Icon(
                                    painter = painterResource(R.drawable.ic_trashcan),
                                    contentDescription = "Remove from album",
                                    tint = Color(0xFFFFA500),
                                    modifier = Modifier.size(12.dp)
                                )
                            }
                        } else {
                            // Gallery context: delete the photo
                            IconButton(onClick = { viewModel.deletePhoto(currentPhoto, onBack) }) {
                                Icon(
                                    painter = painterResource(R.drawable.ic_trashcan),
                                    contentDescription = "Delete",
                                    tint = Color.White,
                                    modifier = Modifier.size(12.dp)
                                )
                            }
                        }
                    }
                }
            }
        }

        // ── Tag bar overlay (bottom) ───────────────────────────────────
        if (isPlainMode && currentPhoto?.serverPhotoId != null) {
            androidx.compose.animation.AnimatedVisibility(
                visible = showOverlay,
                enter = androidx.compose.animation.fadeIn(),
                exit = androidx.compose.animation.fadeOut(),
                modifier = Modifier.align(Alignment.BottomCenter)
            ) {
                Surface(
                    color = Color.Black.copy(alpha = 0.6f),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .navigationBarsPadding()
                            .padding(horizontal = 12.dp, vertical = 8.dp)
                    ) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(6.dp)
                        ) {
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
                            if (showTagInput) {
                                OutlinedTextField(
                                    value = tagInputText,
                                    onValueChange = { if (it.length <= 100) tagInputText = it },
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

        // ── Info panel (slide up from bottom) ─────────────────────────
        androidx.compose.animation.AnimatedVisibility(
            visible = showInfoPanel,
            enter = androidx.compose.animation.slideInVertically { it },
            exit = androidx.compose.animation.slideOutVertically { it },
            modifier = Modifier.align(Alignment.BottomCenter)
        ) {
            Surface(
                color = Color(0xF2111827),
                shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
                modifier = Modifier.fillMaxWidth()
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .navigationBarsPadding()
                        .padding(horizontal = 20.dp, vertical = 16.dp)
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text("Photo Details", color = Color.White, fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
                        IconButton(onClick = { showInfoPanel = false }, modifier = Modifier.size(24.dp)) {
                            Icon(Icons.Default.Close, contentDescription = "Close", tint = Color.Gray, modifier = Modifier.size(16.dp))
                        }
                    }
                    Spacer(Modifier.height(12.dp))
                    currentPhoto?.let { photo ->
                        InfoDetailRow("Filename", photo.filename)
                        InfoDetailRow("Type", photo.mimeType)
                        if (photo.width > 0 && photo.height > 0) {
                            InfoDetailRow("Dimensions", "${photo.width} × ${photo.height}")
                        }
                        photo.sizeBytes?.let { size ->
                            InfoDetailRow("Size", formatInfoBytes(size))
                        }
                        if (photo.takenAt > 0L) {
                            InfoDetailRow("Taken", java.text.SimpleDateFormat("MMM d, yyyy  h:mm a", java.util.Locale.getDefault()).format(java.util.Date(photo.takenAt)))
                        }
                        if (photo.createdAt > 0L) {
                            InfoDetailRow("Uploaded", java.text.SimpleDateFormat("MMM d, yyyy  h:mm a", java.util.Locale.getDefault()).format(java.util.Date(photo.createdAt)))
                        }
                        photo.durationSecs?.let { dur ->
                            InfoDetailRow("Duration", "%.1fs".format(dur))
                        }
                        photo.cameraModel?.let { cam ->
                            InfoDetailRow("Device", cam)
                        }
                        if (photo.latitude != null && photo.longitude != null) {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(vertical = 4.dp),
                                horizontalArrangement = Arrangement.SpaceBetween
                            ) {
                                Text("Location", color = Color.Gray, fontSize = 13.sp)
                                Text(
                                    "%.5f, %.5f ↗".format(photo.latitude, photo.longitude),
                                    color = Color(0xFF60A5FA),
                                    fontSize = 13.sp,
                                    modifier = Modifier.clickable {
                                        val uri = Uri.parse(
                                            "https://www.google.com/maps?q=${photo.latitude},${photo.longitude}"
                                        )
                                        context.startActivity(
                                            android.content.Intent(android.content.Intent.ACTION_VIEW, uri)
                                        )
                                    }
                                )
                            }
                        }
                    }
                }
            }
        }

        // ── Edit mode panel (bottom) ──────────────────────────────────
        androidx.compose.animation.AnimatedVisibility(
            visible = editMode,
            enter = androidx.compose.animation.slideInVertically { it },
            exit = androidx.compose.animation.slideOutVertically { it },
            modifier = Modifier.align(Alignment.BottomCenter)
        ) {
            Surface(
                color = Color(0xE6111827),
                modifier = Modifier.fillMaxWidth()
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .navigationBarsPadding()
                        .padding(horizontal = 16.dp, vertical = 12.dp)
                ) {
                    // Tab selector
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.Center
                    ) {
                        listOf("crop" to "Crop", "brightness" to "Brightness").forEach { (key, label) ->
                            TextButton(onClick = { editTab = key }) {
                                Text(
                                    label,
                                    color = if (editTab == key) Color(0xFF60A5FA) else Color.Gray,
                                    fontSize = 14.sp
                                )
                            }
                        }
                    }

                    Spacer(Modifier.height(8.dp))

                    when (editTab) {
                        "brightness" -> {
                            Text("Brightness: ${brightnessValue.toInt()}", color = Color.White, fontSize = 12.sp)
                            Slider(
                                value = brightnessValue,
                                onValueChange = { brightnessValue = it },
                                valueRange = -100f..100f,
                                modifier = Modifier.fillMaxWidth(),
                                colors = SliderDefaults.colors(
                                    thumbColor = Color(0xFF60A5FA),
                                    activeTrackColor = Color(0xFF3B82F6)
                                )
                            )
                        }
                        "crop" -> {
                            // Crop overlay is drawn on the main photo above; just show instructions here
                            Text("Drag the corner handles on the photo to adjust crop", color = Color.Gray, fontSize = 12.sp)
                        }
                    }

                    Spacer(Modifier.height(8.dp))

                    // Action buttons
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceEvenly
                    ) {
                        TextButton(onClick = { clearCrop() }) {
                            Text("Reset", color = Color(0xFFF87171), fontSize = 14.sp)
                        }
                        TextButton(onClick = { editMode = false }) {
                            Text("Cancel", color = Color.Gray, fontSize = 14.sp)
                        }
                        Button(
                            onClick = { saveEdit() },
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF3B82F6))
                        ) {
                            Text("Save", color = Color.White, fontSize = 14.sp)
                        }
                    }
                }
            }
        }

        // ── Download confirmation snackbar ─────────────────────────────
        downloadMessage?.let { msg ->
            LaunchedEffect(msg) {
                kotlinx.coroutines.delay(2000)
                downloadMessage = null
            }
            Snackbar(
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .padding(16.dp)
            ) {
                Text(msg)
            }
        }
    }
}
