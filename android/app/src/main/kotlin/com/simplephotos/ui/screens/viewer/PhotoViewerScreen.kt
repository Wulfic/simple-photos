/**
 * Full-screen photo/video viewer with horizontal paging.
 *
 * Supports pinch-to-zoom, swipe navigation, share/download/delete actions,
 * photo info panel, tag management, crop editing, and encrypted-mode
 * (decrypt-to-memory) rendering.
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
import androidx.compose.ui.zIndex
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
import com.simplephotos.ui.components.RenderingCopyBanner
import kotlinx.coroutines.launch
import org.json.JSONObject

private const val TAG = "PhotoViewerScreen"

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
        Log.d(TAG, "[enterEditMode] photo=${photo.localId}, server=${photo.serverPhotoId}, " +
            "dims=${photo.width}×${photo.height}, mediaType=${photo.mediaType}, " +
            "cropMetadata=${photo.cropMetadata}")
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
        Log.d(TAG, "[saveEdit] photo=${photo.localId}, crop=(${cropCorners.x},${cropCorners.y},${cropCorners.w},${cropCorners.h}), " +
            "rotate=$rotateValue, brightness=$brightnessValue, trimStart=$trimStart, trimEnd=$trimEnd")
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
        Log.d(TAG, "[saveCopy] photo=${photo.localId}, crop=(${cropCorners.x},${cropCorners.y},${cropCorners.w},${cropCorners.h}), " +
            "rotate=$rotateValue, brightness=$brightnessValue, trimStart=$trimStart, trimEnd=$trimEnd")
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
        Log.d(TAG, "[clearCrop] photo=${photo.localId}")
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
                        if (currentPhoto.serverPhotoId != null) {
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

        // ── Rendering copy banner (shown while server renders an edited copy) ─
        Box(
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .padding(start = 16.dp, end = 16.dp, bottom = 24.dp)
                .zIndex(50f)
        ) {
            RenderingCopyBanner(visible = viewModel.isRenderingCopy)
        }

        // ── Info panel (slide up from bottom) ─────────────────────────
        ViewerInfoPanel(
            visible = showInfoPanel,
            photo = currentPhoto,
            onDismiss = { showInfoPanel = false },
            modifier = Modifier.align(Alignment.BottomCenter)
        )

        // ── Edit mode panel (bottom) ──────────────────────────────────
        ViewerEditPanel(
            visible = editMode,
            editTab = editTab,
            onEditTabChange = { editTab = it },
            brightnessValue = brightnessValue,
            onBrightnessChange = { brightnessValue = it },
            rotateValue = rotateValue,
            onRotateLeft = { rotateValue = (rotateValue + 270) % 360 },
            onRotateRight = { rotateValue = (rotateValue + 90) % 360 },
            trimStart = trimStart,
            onTrimStartChange = { trimStart = it },
            trimEnd = trimEnd,
            onTrimEndChange = { trimEnd = it },
            mediaDuration = mediaDuration,
            currentPhoto = currentPhoto,
            onSave = { saveEdit() },
            onSaveCopy = { saveCopy() },
            onReset = { clearCrop() },
            onCancel = { editMode = false },
            modifier = Modifier.align(Alignment.BottomCenter)
        )

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

