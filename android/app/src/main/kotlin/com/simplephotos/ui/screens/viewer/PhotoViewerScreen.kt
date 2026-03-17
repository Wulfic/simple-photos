/**
 * Full-screen photo/video viewer with horizontal paging.
 *
 * Supports pinch-to-zoom, swipe navigation, share/download/delete actions,
 * photo info panel, tag management, crop editing, and both plain-mode
 * (authenticated URL) and encrypted-mode (decrypt-to-memory) rendering.
 */
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
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
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
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
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
// ---------------------------------------------------------------------------

/**
 * Full-screen photo/video viewer with horizontal swipe paging.
 *
 * Supports pinch-to-zoom, video playback, EXIF info, favorites,
 * crop/edit mode, and detail overlays.
 */
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

    // ── Single shared ExoPlayer ──────────────────────────────────────
    // One ExoPlayer for the entire viewer. All videos are downloaded to
    // temp files BEFORE playback starts (see PhotoPageContent), so the
    // player only ever reads local file:// URIs.  This means:
    //   • No OkHttp / HTTP buffering in the Java heap during playback
    //   • No CacheDataSource / SimpleCache complexity
    //   • The heap is free for the MediaCodec output buffers
    // This mirrors how the web browser works: download first, play local.
    val sharedPlayer = remember {
        run {
            // Local-only data source — handles file:// and content:// URIs.
            // No network data source needed; download is handled separately.
            val mediaSourceFactory = DefaultMediaSourceFactory(
                DefaultDataSource.Factory(context)
            )

            // ── Minimal in-memory buffer ─────────────────────────────
            // Reading from local storage is fast (100+ MB/s), so we only
            // need a tiny buffer.  This leaves maximum heap for the
            // MediaCodec output frame buffers (~50–100 MB for 4K).
            val heapBytes = Runtime.getRuntime().maxMemory()
            Log.d("VideoPlayer", "Heap=${heapBytes/1024/1024}MB — local-only playback, heap buffer=2.5 MB")

            val loadControl = DefaultLoadControl.Builder()
                .setBufferDurationsMs(
                    /* minBufferMs */        2_500,
                    /* maxBufferMs */       30_000,
                    /* bufferForPlaybackMs */  1_000,
                    /* bufferForPlaybackAfterRebufferMs */ 2_000
                )
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
    // Release the shared player when the viewer leaves composition
    DisposableEffect(Unit) {
        onDispose {
            sharedPlayer.stop()
            sharedPlayer.release()
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
    var editTab by remember { mutableStateOf("crop") } // "crop" | "brightness" | "rotate" | "trim"
    var cropCorners by remember { mutableStateOf(CropCorners(0f, 0f, 1f, 1f)) }
    var brightnessValue by remember { mutableStateOf(0f) }
    var rotateValue by remember { mutableStateOf(0) } // 0, 90, 180, 270
    var trimStart by remember { mutableStateOf(0f) }
    var trimEnd by remember { mutableStateOf(0f) }
    var mediaDuration by remember { mutableStateOf(0f) }
    var mediaIntrinsicWidth by remember { mutableStateOf(-1f) }
    var mediaIntrinsicHeight by remember { mutableStateOf(-1f) }

    // Load saved trim values when page changes (for non-edit mode playback)
    LaunchedEffect(pagerState.currentPage) {
        mediaIntrinsicWidth = -1f
        mediaIntrinsicHeight = -1f
        val photo = viewModel.allPhotos.getOrNull(pagerState.currentPage)
        val cm = photo?.cropMetadata
        if (cm != null) {
            try {
                val json = JSONObject(cm)
                trimStart = json.optDouble("trimStart", 0.0).toFloat()
                trimEnd = json.optDouble("trimEnd", 0.0).toFloat()
            } catch (_: Exception) {
                trimStart = 0f
                trimEnd = 0f
            }
        } else {
            trimStart = 0f
            trimEnd = 0f
        }
        mediaDuration = photo?.durationSecs ?: 0f
    }

    // Initialize edit mode from existing crop metadata
    fun enterEditMode() {
        val photo = currentPhoto ?: return
        val cm = photo.cropMetadata
        // Use photo duration as fallback for trim end
        val dur = photo.durationSecs ?: 0f
        if (dur > 0f && mediaDuration <= 0f) mediaDuration = dur

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
                rotateValue = json.optInt("rotate", 0)
                trimStart = json.optDouble("trimStart", 0.0).toFloat()
                val savedEnd = json.optDouble("trimEnd", 0.0).toFloat()
                trimEnd = if (savedEnd > 0f) savedEnd else mediaDuration
            } catch (_: Exception) {
                cropCorners = CropCorners(0f, 0f, 1f, 1f)
                brightnessValue = 0f
                rotateValue = 0
                trimStart = 0f
                trimEnd = mediaDuration
            }
        } else {
            cropCorners = CropCorners(0f, 0f, 1f, 1f)
            brightnessValue = 0f
            rotateValue = 0
            trimStart = 0f
            trimEnd = mediaDuration
        }
        // Select default tab based on media type (match web behavior)
        val isMedia = photo.mediaType == "video" || photo.mediaType == "audio"
        editTab = if (isMedia) "trim" else "crop"
        editMode = true
    }

    fun saveEdit() {
        val photo = currentPhoto ?: return
        val c = cropCorners
        val isDefaultCrop = c.x <= 0.01f && c.y <= 0.01f && c.w >= 0.99f && c.h >= 0.99f
        val isDefaultBrightness = kotlin.math.abs(brightnessValue) < 1f
        val isDefaultRotate = rotateValue == 0
        val isDefaultTrim = trimStart <= 0.01f &&
            (mediaDuration <= 0f || kotlin.math.abs(trimEnd - mediaDuration) < 0.5f)
        val allDefault = isDefaultCrop && isDefaultBrightness && isDefaultRotate && isDefaultTrim

        if (allDefault) {
            viewModel.saveCropMetadata(photo, null)
        } else {
            val meta = JSONObject().apply {
                put("x", c.x.toDouble().coerceIn(0.0, 1.0))
                put("y", c.y.toDouble().coerceIn(0.0, 1.0))
                put("width", c.w.toDouble().coerceIn(0.05, 1.0))
                put("height", c.h.toDouble().coerceIn(0.05, 1.0))
                put("rotate", rotateValue)
                put("brightness", brightnessValue.toDouble())
                if (!isDefaultTrim) {
                    put("trimStart", trimStart.toDouble())
                    put("trimEnd", trimEnd.toDouble())
                }
            }.toString()
            viewModel.saveCropMetadata(photo, meta)
        }
        editMode = false
    }

    fun saveCopy() {
        val photo = currentPhoto ?: return
        val c = cropCorners
        val isDefaultCrop = c.x <= 0.01f && c.y <= 0.01f && c.w >= 0.99f && c.h >= 0.99f
        val isDefaultBrightness = kotlin.math.abs(brightnessValue) < 1f
        val isDefaultRotate = rotateValue == 0
        val isDefaultTrim = trimStart <= 0.01f &&
            (mediaDuration <= 0f || kotlin.math.abs(trimEnd - mediaDuration) < 0.5f)
        val allDefault = isDefaultCrop && isDefaultBrightness && isDefaultRotate && isDefaultTrim

        val meta = if (allDefault) null else JSONObject().apply {
            put("x", c.x.toDouble().coerceIn(0.0, 1.0))
            put("y", c.y.toDouble().coerceIn(0.0, 1.0))
            put("width", c.w.toDouble().coerceIn(0.05, 1.0))
            put("height", c.h.toDouble().coerceIn(0.05, 1.0))
            put("rotate", rotateValue)
            put("brightness", brightnessValue.toDouble())
            if (!isDefaultTrim) {
                put("trimStart", trimStart.toDouble())
                put("trimEnd", trimEnd.toDouble())
            }
        }.toString()
        viewModel.duplicatePhoto(photo, meta) {
            editMode = false
        }
    }

    fun clearCrop() {
        val photo = currentPhoto ?: return
        viewModel.saveCropMetadata(photo, null)
        cropCorners = CropCorners(0f, 0f, 1f, 1f)
        brightnessValue = 0f
        rotateValue = 0
        trimStart = 0f
        trimEnd = mediaDuration
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
        val cropPadding by androidx.compose.animation.core.animateDpAsState(
            targetValue = if (editMode && editTab == "crop") 32.dp else 0.dp
        )

        // ── Full-screen pager (behind overlays) ────────────────────────
        HorizontalPager(
            state = pagerState,
            modifier = Modifier.fillMaxSize().padding(cropPadding),
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
                    editRotation = rotateValue,
                    trimStart = trimStart,
                    trimEnd = trimEnd,
                    sharedPlayer = sharedPlayer,
                    activeVideoUri = activeVideoUri,
                    intrinsicWidth = if (pagerState.currentPage == page) mediaIntrinsicWidth else -1f,
                    intrinsicHeight = if (pagerState.currentPage == page) mediaIntrinsicHeight else -1f,
                    onVideoUriReady = { uri, filename ->
                        // Load the new media item into the shared player.
                        // Only swap if the URI actually changed (avoids re-prepare
                        // on every recomposition).
                        if (uri != activeVideoUri) {
                            sharedPlayerError = null
                            sharedPlayerConverting = needsWebPreview(filename) != null
                            activeVideoUri = uri

                            // Evict only full-resolution viewer images from Coil's
                            // memory cache (URLs ending in /web).  Gallery thumbnails
                            // (URLs ending in /thumb, keyed at 256px) are preserved
                            // so they display instantly when the user navigates back.
                            context.imageLoader.memoryCache?.let { cache ->
                                val keysToRemove = cache.keys.filter { key ->
                                    val data = key.key
                                    data.contains("/web") || data.contains("/file")
                                }
                                keysToRemove.forEach { cache.remove(it) }
                            }

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
                    onDurationKnown = { dur ->
                        if (dur > 0f) {
                            mediaDuration = dur
                            // Initialize trimEnd to full duration if not yet set
                            if (trimEnd <= 0f) trimEnd = dur
                        }
                    },
                    playerError = sharedPlayerError,
                    isConverting = sharedPlayerConverting,
                    onMediaSizeLoaded = { w, h ->
                        if (pagerState.currentPage == page) {
                            mediaIntrinsicWidth = w
                            mediaIntrinsicHeight = h
                        }
                    }
                )
            }
        }

        // ── Crop overlay drawn on top of the photo/video ─────────────────
        // Uses actual media bounds computed from photo dimensions + container
        // constraints so the crop handles align with the rendered image edges
        // (like getBoundingClientRect() in the web version).
        // When rotated 90/270°, the effective aspect ratio is swapped.
        if (editMode && editTab == "crop") {
            BoxWithConstraints(
                modifier = Modifier.fillMaxSize().padding(cropPadding)
            ) {
                val cc = cropCorners

                // Container dimensions in pixels (same coordinate space as Canvas)
                val cW = constraints.maxWidth.toFloat()
                val cH = constraints.maxHeight.toFloat()

                // Use dynamically retrieved intrinsic size if available, fallback to DB
                val rawW = if (mediaIntrinsicWidth > 0f) mediaIntrinsicWidth else (currentPhoto?.width ?: 0).toFloat()
                val rawH = if (mediaIntrinsicHeight > 0f) mediaIntrinsicHeight else (currentPhoto?.height ?: 0).toFloat()

                // Account for rotation applied manually in edit mode: 90/270 swaps effective width↔height
                val rot = rotateValue % 360
                val isSwapped = rot == 90 || rot == 270 || rot == -90 || rot == -270
                val photoW = if (isSwapped) rawH else rawW
                val photoH = if (isSwapped) rawW else rawH

                val mL: Float   // media left offset in pixels
                val mT: Float   // media top offset in pixels
                val mW: Float   // media rendered width in pixels
                val mH: Float   // media rendered height in pixels
                if (photoW > 0f && photoH > 0f && cW > 0f && cH > 0f) {
                    val imgAspect = photoW / photoH
                    val containerAspect = cW / cH
                    if (imgAspect > containerAspect) {
                        mW = cW; mH = cW / imgAspect
                    } else {
                        mH = cH; mW = cH * imgAspect
                    }
                    mL = (cW - mW) / 2f
                    mT = (cH - mH) / 2f
                } else {
                    mL = 0f; mT = 0f; mW = cW; mH = cH
                }

                // Visual dot = 16dp, touch target = 40dp (easy to grab on mobile)
                val dotRadius = 8.dp
                val touchRadius = 20.dp

                // Darkened overlay outside crop + white border lines + corner dots
                Canvas(modifier = Modifier.fillMaxSize()) {
                    val left = mL + cc.x * mW
                    val top = mT + cc.y * mH
                    val right = mL + (cc.x + cc.w) * mW
                    val bottom = mT + (cc.y + cc.h) * mH

                    val dimColor = Color.Black.copy(alpha = 0.5f)
                    // Top
                    drawRect(dimColor, topLeft = Offset.Zero, size = androidx.compose.ui.geometry.Size(size.width, top))
                    // Bottom
                    drawRect(dimColor, topLeft = Offset(0f, bottom), size = androidx.compose.ui.geometry.Size(size.width, size.height - bottom))
                    // Left
                    drawRect(dimColor, topLeft = Offset(0f, top), size = androidx.compose.ui.geometry.Size(left, bottom - top))
                    // Right
                    drawRect(dimColor, topLeft = Offset(right, top), size = androidx.compose.ui.geometry.Size(size.width - right, bottom - top))

                    // White border lines
                    val strokeW = 2f
                    drawLine(Color.White, Offset(left, top), Offset(right, top), strokeWidth = strokeW)
                    drawLine(Color.White, Offset(right, top), Offset(right, bottom), strokeWidth = strokeW)
                    drawLine(Color.White, Offset(right, bottom), Offset(left, bottom), strokeWidth = strokeW)
                    drawLine(Color.White, Offset(left, bottom), Offset(left, top), strokeWidth = strokeW)

                    // 4 white corner dots
                    val dotRadiusPx = dotRadius.toPx()
                    for (center in listOf(
                        Offset(left, top), Offset(right, top),
                        Offset(left, bottom), Offset(right, bottom)
                    )) {
                        drawCircle(Color.Black.copy(alpha = 0.4f), radius = dotRadiusPx + 2f, center = center)
                        drawCircle(Color.White, radius = dotRadiusPx, center = center)
                    }
                }

                // Invisible touch targets for the 4 corners
                val cornerDefs = listOf(
                    "tl" to Offset(mL + cc.x * mW, mT + cc.y * mH),
                    "tr" to Offset(mL + (cc.x + cc.w) * mW, mT + cc.y * mH),
                    "bl" to Offset(mL + cc.x * mW, mT + (cc.y + cc.h) * mH),
                    "br" to Offset(mL + (cc.x + cc.w) * mW, mT + (cc.y + cc.h) * mH)
                )

                for ((corner, pos) in cornerDefs) {
                    Box(
                        modifier = Modifier
                            .offset(
                                x = with(androidx.compose.ui.platform.LocalDensity.current) { pos.x.toDp() } - touchRadius,
                                y = with(androidx.compose.ui.platform.LocalDensity.current) { pos.y.toDp() } - touchRadius
                            )
                            .size(touchRadius * 2)
                            .pointerInput(corner, mW, mH) {
                                detectDragGestures { change, dragAmount ->
                                    change.consume()
                                    val minSize = 0.05f
                                    val dx = dragAmount.x / mW
                                    val dy = dragAmount.y / mH
                                    cropCorners = when (corner) {
                                        "tl" -> {
                                            val newX = (cropCorners.x + dx).coerceIn(0f, cropCorners.x + cropCorners.w - minSize)
                                            val newY = (cropCorners.y + dy).coerceIn(0f, cropCorners.y + cropCorners.h - minSize)
                                            CropCorners(
                                                x = newX, y = newY,
                                                w = cropCorners.w + (cropCorners.x - newX),
                                                h = cropCorners.h + (cropCorners.y - newY)
                                            )
                                        }
                                        "tr" -> {
                                            val newR = (cropCorners.x + cropCorners.w + dx).coerceIn(cropCorners.x + minSize, 1f)
                                            val newY = (cropCorners.y + dy).coerceIn(0f, cropCorners.y + cropCorners.h - minSize)
                                            CropCorners(
                                                x = cropCorners.x, y = newY,
                                                w = newR - cropCorners.x,
                                                h = cropCorners.h + (cropCorners.y - newY)
                                            )
                                        }
                                        "bl" -> {
                                            val newX = (cropCorners.x + dx).coerceIn(0f, cropCorners.x + cropCorners.w - minSize)
                                            val newB = (cropCorners.y + cropCorners.h + dy).coerceIn(cropCorners.y + minSize, 1f)
                                            CropCorners(
                                                x = newX, y = cropCorners.y,
                                                w = cropCorners.w + (cropCorners.x - newX),
                                                h = newB - cropCorners.y
                                            )
                                        }
                                        "br" -> {
                                            val newR = (cropCorners.x + cropCorners.w + dx).coerceIn(cropCorners.x + minSize, 1f)
                                            val newB = (cropCorners.y + cropCorners.h + dy).coerceIn(cropCorners.y + minSize, 1f)
                                            CropCorners(
                                                x = cropCorners.x, y = cropCorners.y,
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
                        // Edit button — available for photos, videos, and audio
                        if (currentPhoto.mediaType == "photo" || currentPhoto.mediaType == "video" || currentPhoto.mediaType == "audio") {
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
                                    val destUri = context.contentResolver.insert(
                                        MediaStore.Downloads.EXTERNAL_CONTENT_URI, values
                                    )
                                    if (destUri == null) {
                                        downloadMessage = "Download failed"
                                        return@launch
                                    }

                                    val saved = when {
                                        // Local files: stream from content resolver
                                        photo.localPath != null -> {
                                            try {
                                                context.contentResolver.openInputStream(Uri.parse(photo.localPath))?.use { input ->
                                                    context.contentResolver.openOutputStream(destUri)?.use { output ->
                                                        input.copyTo(output)
                                                    }
                                                }
                                                true
                                            } catch (_: Exception) { false }
                                        }
                                        // Server files: stream download → temp file → MediaStore
                                        // Uses downloadPhotoToFile() which streams to disk with
                                        // constant ~8 KB heap — safe for large videos.
                                        else -> {
                                            val tempFile = java.io.File.createTempFile("save_", ".tmp", context.cacheDir)
                                            try {
                                                val ok = viewModel.downloadPhotoToFile(photo, tempFile)
                                                if (ok) {
                                                    tempFile.inputStream().buffered().use { input ->
                                                        context.contentResolver.openOutputStream(destUri)?.use { output ->
                                                            input.copyTo(output)
                                                        }
                                                    }
                                                }
                                                ok
                                            } finally {
                                                tempFile.delete()
                                            }
                                        }
                                    }

                                    downloadMessage = if (saved) "Saved to Downloads" else "Download failed"
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
            val currentMediaType = currentPhoto?.mediaType ?: "photo"
            val isPhoto = currentMediaType == "photo"
            val isVideo = currentMediaType == "video"
            val isAudio = currentMediaType == "audio"
            val showCrop = isPhoto || isVideo
            val showBrightness = isPhoto || isVideo
            val showRotate = isPhoto || isVideo
            val showTrim = isVideo || isAudio

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
                    // Tab selector — only show tabs available for this media type
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(bottom = 8.dp),
                        horizontalArrangement = Arrangement.Center,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        if (showCrop) {
                            Surface(
                                shape = RoundedCornerShape(50),
                                color = if (editTab == "crop") Color.White else Color.Transparent,
                                modifier = Modifier
                                    .padding(horizontal = 4.dp)
                                    .clip(RoundedCornerShape(50))
                                    .clickable { editTab = "crop" }
                            ) {
                                Text(
                                    "Crop",
                                    color = if (editTab == "crop") Color.Black else Color.White,
                                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                    fontSize = 14.sp,
                                    fontWeight = FontWeight.Medium
                                )
                            }
                        }
                        if (showBrightness) {
                            Surface(
                                shape = RoundedCornerShape(50),
                                color = if (editTab == "brightness") Color.White else Color.Transparent,
                                modifier = Modifier
                                    .padding(horizontal = 4.dp)
                                    .clip(RoundedCornerShape(50))
                                    .clickable { editTab = "brightness" }
                            ) {
                                Text(
                                    "Brightness",
                                    color = if (editTab == "brightness") Color.Black else Color.White,
                                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                    fontSize = 14.sp,
                                    fontWeight = FontWeight.Medium
                                )
                            }
                        }
                        if (showRotate) {
                            Surface(
                                shape = RoundedCornerShape(50),
                                color = if (editTab == "rotate") Color.White else Color.Transparent,
                                modifier = Modifier
                                    .padding(horizontal = 4.dp)
                                    .clip(RoundedCornerShape(50))
                                    .clickable { editTab = "rotate" }
                            ) {
                                Text(
                                    "Rotate",
                                    color = if (editTab == "rotate") Color.Black else Color.White,
                                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                    fontSize = 14.sp,
                                    fontWeight = FontWeight.Medium
                                )
                            }
                        }
                        if (showTrim) {
                            Surface(
                                shape = RoundedCornerShape(50),
                                color = if (editTab == "trim") Color.White else Color.Transparent,
                                modifier = Modifier
                                    .padding(horizontal = 4.dp)
                                    .clip(RoundedCornerShape(50))
                                    .clickable { editTab = "trim" }
                            ) {
                                Text(
                                    "Trim",
                                    color = if (editTab == "trim") Color.Black else Color.White,
                                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                    fontSize = 14.sp,
                                    fontWeight = FontWeight.Medium
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
                        "rotate" -> {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.Center,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                // Rotate left button
                                IconButton(onClick = { rotateValue = (rotateValue + 270) % 360 }) {
                                    Icon(
                                        painter = painterResource(R.drawable.ic_back_arrow),
                                        contentDescription = "Rotate left",
                                        tint = Color.White,
                                        modifier = Modifier.size(20.dp)
                                    )
                                }
                                Spacer(Modifier.width(16.dp))
                                Text(
                                    "${rotateValue}°",
                                    color = Color.White,
                                    fontSize = 14.sp,
                                    fontWeight = FontWeight.Medium
                                )
                                Spacer(Modifier.width(16.dp))
                                // Rotate right button
                                IconButton(onClick = { rotateValue = (rotateValue + 90) % 360 }) {
                                    Icon(
                                        painter = painterResource(R.drawable.ic_back_arrow),
                                        contentDescription = "Rotate right",
                                        tint = Color.White,
                                        modifier = Modifier
                                            .size(20.dp)
                                            .graphicsLayer(scaleX = -1f)
                                    )
                                }
                            }
                        }
                        "trim" -> {
                            if (mediaDuration > 0f) {
                                // Time labels
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    horizontalArrangement = Arrangement.SpaceBetween
                                ) {
                                    Text(formatTrimTime(trimStart), color = Color.Gray, fontSize = 12.sp)
                                    Text(
                                        "${formatTrimTime(trimEnd - trimStart)} selected",
                                        color = Color.White,
                                        fontSize = 12.sp,
                                        fontWeight = FontWeight.Medium
                                    )
                                    Text(formatTrimTime(trimEnd), color = Color.Gray, fontSize = 12.sp)
                                }
                                Spacer(Modifier.height(4.dp))

                                // ── Single-bar dual-thumb trim slider (matches web) ──
                                val thumbRadiusDp = 8.dp
                                val trackHeightDp = 6.dp

                                BoxWithConstraints(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .height(40.dp)
                                ) {
                                    val trackW = constraints.maxWidth.toFloat()
                                    val density = androidx.compose.ui.platform.LocalDensity.current

                                    // Background track
                                    Box(
                                        modifier = Modifier
                                            .fillMaxWidth()
                                            .height(trackHeightDp)
                                            .align(Alignment.Center)
                                            .clip(RoundedCornerShape(50))
                                            .background(Color.White.copy(alpha = 0.1f))
                                    )

                                    // Selected range highlight (blue bar between start and end)
                                    val startFrac = (trimStart / mediaDuration).coerceIn(0f, 1f)
                                    val endFrac = (trimEnd / mediaDuration).coerceIn(0f, 1f)
                                    Box(
                                        modifier = Modifier
                                            .height(trackHeightDp)
                                            .align(Alignment.CenterStart)
                                            .offset(x = with(density) { (startFrac * trackW).toDp() })
                                            .width(with(density) { ((endFrac - startFrac) * trackW).toDp() })
                                            .clip(RoundedCornerShape(50))
                                            .background(Color(0xFF3B82F6).copy(alpha = 0.6f))
                                    )

                                    // Start thumb
                                    val startThumbX = with(density) { (startFrac * trackW).toDp() }
                                    Box(
                                        modifier = Modifier
                                            .offset(x = startThumbX - thumbRadiusDp, y = 0.dp)
                                            .size(thumbRadiusDp * 2)
                                            .align(Alignment.CenterStart)
                                            .clip(CircleShape)
                                            .background(Color.White)
                                            .then(
                                                Modifier.drawBehind {
                                                    drawCircle(
                                                        color = Color(0xFF3B82F6),
                                                        radius = size.minDimension / 2f,
                                                        style = Stroke(width = 2.dp.toPx())
                                                    )
                                                }
                                            )
                                            .pointerInput(mediaDuration) {
                                                detectDragGestures { change, dragAmount ->
                                                    change.consume()
                                                    val dx = dragAmount.x / trackW
                                                    trimStart = (trimStart + dx * mediaDuration)
                                                        .coerceIn(0f, trimEnd - 0.5f)
                                                }
                                            }
                                    )

                                    // End thumb
                                    val endThumbX = with(density) { (endFrac * trackW).toDp() }
                                    Box(
                                        modifier = Modifier
                                            .offset(x = endThumbX - thumbRadiusDp, y = 0.dp)
                                            .size(thumbRadiusDp * 2)
                                            .align(Alignment.CenterStart)
                                            .clip(CircleShape)
                                            .background(Color.White)
                                            .then(
                                                Modifier.drawBehind {
                                                    drawCircle(
                                                        color = Color(0xFF3B82F6),
                                                        radius = size.minDimension / 2f,
                                                        style = Stroke(width = 2.dp.toPx())
                                                    )
                                                }
                                            )
                                            .pointerInput(mediaDuration) {
                                                detectDragGestures { change, dragAmount ->
                                                    change.consume()
                                                    val dx = dragAmount.x / trackW
                                                    trimEnd = (trimEnd + dx * mediaDuration)
                                                        .coerceIn(trimStart + 0.5f, mediaDuration)
                                                }
                                            }
                                    )
                                }

                                Spacer(Modifier.height(4.dp))
                                Text(
                                    "Full duration: ${formatTrimTime(mediaDuration)}",
                                    color = Color.Gray.copy(alpha = 0.6f),
                                    fontSize = 11.sp,
                                    modifier = Modifier.fillMaxWidth(),
                                    textAlign = androidx.compose.ui.text.style.TextAlign.Center
                                )
                            } else {
                                // Duration not yet known
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    horizontalArrangement = Arrangement.Center,
                                    verticalAlignment = Alignment.CenterVertically
                                ) {
                                    CircularProgressIndicator(
                                        color = Color.Gray,
                                        modifier = Modifier.size(16.dp),
                                        strokeWidth = 2.dp
                                    )
                                    Spacer(Modifier.width(8.dp))
                                    Text("Loading duration…", color = Color.Gray, fontSize = 12.sp)
                                }
                                Spacer(Modifier.height(4.dp))
                                Text(
                                    "Play the media briefly if the duration doesn't appear.",
                                    color = Color.Gray.copy(alpha = 0.5f),
                                    fontSize = 11.sp,
                                    modifier = Modifier.fillMaxWidth(),
                                    textAlign = androidx.compose.ui.text.style.TextAlign.Center
                                )
                            }
                        }
                    }

                    Spacer(Modifier.height(8.dp))

                    // Action buttons — matches web: Save, Save Copy, Reset, Cancel
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(bottom = 8.dp),
                        horizontalArrangement = Arrangement.Center
                    ) {
                        Button(
                            onClick = { saveEdit() },
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF2563EB)),
                            shape = RoundedCornerShape(8.dp),
                            modifier = Modifier.padding(horizontal = 4.dp)
                        ) {
                            Text("Save", color = Color.White, fontSize = 14.sp)
                        }
                        Button(
                            onClick = { saveCopy() },
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF16A34A)),
                            shape = RoundedCornerShape(8.dp),
                            modifier = Modifier.padding(horizontal = 4.dp)
                        ) {
                            Text("Save Copy", color = Color.White, fontSize = 14.sp)
                        }
                        if (currentPhoto?.cropMetadata != null) {
                            Button(
                                onClick = { clearCrop() },
                                colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF4B5563)),
                                shape = RoundedCornerShape(8.dp),
                                modifier = Modifier.padding(horizontal = 4.dp)
                            ) {
                                Text("Reset", color = Color.White, fontSize = 14.sp)
                            }
                        }
                        Button(
                            onClick = { editMode = false },
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF374151)),
                            shape = RoundedCornerShape(8.dp),
                            modifier = Modifier.padding(horizontal = 4.dp)
                        ) {
                            Text("Cancel", color = Color.White, fontSize = 14.sp)
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

// ── Helpers ──────────────────────────────────────────────────────────────────

/** Format seconds as MM:SS or HH:MM:SS */
private fun formatTrimTime(secs: Float): String {
    val s = secs.coerceAtLeast(0f).toInt()
    val h = s / 3600
    val m = (s % 3600) / 60
    val sec = s % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, sec) else "%d:%02d".format(m, sec)
}

