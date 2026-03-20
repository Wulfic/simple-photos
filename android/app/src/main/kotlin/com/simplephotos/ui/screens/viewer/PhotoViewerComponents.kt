/**
 * Photo viewer sub-components — video player with ExoPlayer, gesture
 * handling (pinch-zoom, double-tap), info panel overlays, and toolbar
 * action buttons used by [PhotoViewerScreen].
 */
package com.simplephotos.ui.screens.viewer

import android.net.Uri
import android.util.Log
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.PhotoEntity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject

// Parsed crop metadata for zoom/brightness/rotation transforms
internal data class CropInfo(
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val brightness: Float,
    val rotate: Int = 0
)

// ---------------------------------------------------------------------------
// Per-page content — each page independently loads and renders its photo
// ---------------------------------------------------------------------------

/**
 * Renders a single pager page: decrypts and displays the photo or video,
 * manages zoom/pan gestures, and hosts the crop/edit overlay when active.
 */
@Composable
internal fun PhotoPageContent(
    photo: PhotoEntity,
    serverBaseUrl: String,
    viewModel: PhotoViewerViewModel,
    okHttpClient: OkHttpClient,
    isActivePage: Boolean = true,
    editMode: Boolean = false,
    editBrightness: Float = 0f,
    editRotation: Int = 0,
    // Trim boundaries (seconds) — applied during playback
    trimStart: Float = 0f,
    trimEnd: Float = 0f,
    // Shared ExoPlayer — owned by PhotoViewerScreen, one instance for all pages
    sharedPlayer: ExoPlayer? = null,
    activeVideoUri: Uri? = null,
    onVideoUriReady: ((Uri, String) -> Unit)? = null,
    onDurationKnown: ((Float) -> Unit)? = null,
    playerError: String? = null,
    onMediaSizeLoaded: ((Float, Float) -> Unit)? = null,
    intrinsicWidth: Float = -1f,
    intrinsicHeight: Float = -1f
) {
    val context = LocalContext.current

    // Determine the content source for this photo
    val hasLocalPath = photo.localPath != null
    val hasEncryptedBlob = photo.serverBlobId != null

    // For encrypted mode, lazily download & decrypt.
    // Photos: store ByteArray for Coil.
    // Videos: write to temp file and store URI to avoid holding large blobs in memory.
    val isMedia = photo.mediaType == "video" || photo.mediaType == "audio"
    var decryptedData by remember(photo.localId) { mutableStateOf<ByteArray?>(null) }
    var tempMediaUri by remember(photo.localId) { mutableStateOf<Uri?>(null) }
    // Show loading spinner when media needs download
    val needsMediaLoad = !hasLocalPath && hasEncryptedBlob
    var decryptLoading by remember(photo.localId) { mutableStateOf(needsMediaLoad) }
    var decryptError by remember(photo.localId) { mutableStateOf<String?>(null) }

    // For videos, gate on isActivePage so we don't download a 50 MB video
    // for the next page while the current page's ExoPlayer is still alive.
    // For photos, download eagerly so Coil can display them during swipe.
    val shouldDecrypt = !hasLocalPath && hasEncryptedBlob &&
        (!isMedia || isActivePage)

    // ── Encrypted video/audio: streaming decrypt-to-file path ─────────
    // Instead of loading the full decoded video into the Java heap (which
    // causes OOM for large files), we stream-decrypt the base64 payload
    // directly to a temp file. Peak heap: ~1× blob size (vs ~4× before).
    LaunchedEffect(photo.localId, shouldDecrypt) {
        if (shouldDecrypt && isMedia && decryptedData == null && tempMediaUri == null) {
            decryptLoading = true
            try {
                val ext = if (photo.mediaType == "audio") ".mp3"
                    else "." + photo.filename.substringAfterLast('.', "mp4").lowercase()
                val uri = withContext(Dispatchers.IO) {
                    val tempFile = java.io.File.createTempFile("media_", ext, context.cacheDir)
                    viewModel.downloadAndDecryptToFile(photo.serverBlobId!!, tempFile)
                    Uri.fromFile(tempFile)
                }
                tempMediaUri = uri
            } catch (e: Throwable) {
                decryptError = e.message ?: "Failed to load media"
            } finally {
                decryptLoading = false
            }
        }
    }

    // ── Encrypted photo: in-memory decrypt path ──────────────────────
    // Photos are small enough to hold in memory; Coil needs a ByteArray.
    LaunchedEffect(photo.localId, shouldDecrypt) {
        if (shouldDecrypt && !isMedia && decryptedData == null && tempMediaUri == null) {
            decryptLoading = true
            try {
                val rawBytes = viewModel.downloadAndDecrypt(photo.serverBlobId!!)
                decryptedData = rawBytes
            } catch (e: Throwable) {
                decryptError = e.message ?: "Failed to load media"
            } finally {
                decryptLoading = false
            }
        }
    }

    // Clean up temp media files when the page leaves composition to prevent
    // accumulating dozens of large video files in the cache directory.
    DisposableEffect(photo.localId) {
        onDispose {
            tempMediaUri?.path?.let { path ->
                try { java.io.File(path).delete() } catch (_: Exception) {}
            }
            // Release decryptedData so GC can reclaim it when page is disposed
            decryptedData = null
            tempMediaUri = null
        }
    }

    // Parse crop metadata for zoom/brightness/rotation display
    val cropInfo = remember(photo.cropMetadata) {
        photo.cropMetadata?.let {
            try {
                val json = JSONObject(it)
                CropInfo(
                    x = json.optDouble("x", 0.0).toFloat(),
                    y = json.optDouble("y", 0.0).toFloat(),
                    width = json.optDouble("width", 1.0).toFloat(),
                    height = json.optDouble("height", 1.0).toFloat(),
                    brightness = json.optDouble("brightness", 0.0).toFloat(),
                    rotate = json.optInt("rotate", 0)
                )
            } catch (_: Exception) { null }
        }
    }

    // Brightness color filter — use live edit value during edit mode, stored value otherwise
    val brightnessFilter = remember(cropInfo?.brightness, editMode, editBrightness) {
        val bValue = if (editMode) editBrightness else (cropInfo?.brightness ?: 0f)
        if (bValue != 0f) {
            val b = 1f + bValue / 100f
            ColorFilter.colorMatrix(ColorMatrix().apply {
                setToScale(b, b, b, 1f)
            })
        } else null
    }

    // Rotation — use live edit value during edit mode, stored value otherwise
    val rotationDegrees = if (editMode) editRotation.toFloat() else (cropInfo?.rotate ?: 0).toFloat()

    // ── Zoom state (double-tap / pinch-to-zoom) ──────────────────────
    var zoomScale by remember { mutableStateOf(1f) }
    var zoomOffsetX by remember { mutableStateOf(0f) }
    var zoomOffsetY by remember { mutableStateOf(0f) }

    BoxWithConstraints(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black)
            .clipToBounds()
            .pointerInput(Unit) {
                // Custom gesture handler: only consume horizontal pans when
                // zoomed in, so that HorizontalPager can still swipe pages
                // at zoom 1×. Pinch-to-zoom (multi-touch) always works.
                // NOTE: key must be Unit (not zoomScale) — using zoomScale
                // as key restarts the coroutine on every scale change,
                // killing the pinch gesture mid-flight.
                awaitEachGesture {
                    val firstDown = awaitFirstDown(requireUnconsumed = false)
                    // Track whether a second pointer arrives (pinch gesture)
                    var isMultiTouch = false
                    var prevCentroid = firstDown.position
                    var initialDist = 0f

                    while (true) {
                        val event = awaitPointerEvent()
                        val pressed = event.changes.filter { it.pressed }
                        if (pressed.isEmpty()) break

                        if (pressed.size >= 2) {
                            isMultiTouch = true
                            val c1 = pressed[0].position
                            val c2 = pressed[1].position
                            val dist = (c1 - c2).getDistance()
                            if (initialDist == 0f) {
                                initialDist = dist
                                prevCentroid = Offset((c1.x + c2.x) / 2f, (c1.y + c2.y) / 2f)
                            }
                            val centroid = Offset((c1.x + c2.x) / 2f, (c1.y + c2.y) / 2f)
                            val zoomFactor = if (initialDist > 0f) dist / initialDist else 1f
                            val pan = centroid - prevCentroid

                            val newScale = (zoomScale * zoomFactor).coerceIn(1f, 5f)
                            if (newScale > 1f) {
                                zoomOffsetX += pan.x
                                zoomOffsetY += pan.y
                            } else {
                                zoomOffsetX = 0f
                                zoomOffsetY = 0f
                            }
                            zoomScale = newScale
                            initialDist = dist
                            prevCentroid = centroid
                            event.changes.forEach { it.consume() }
                        } else if (zoomScale > 1f) {
                            // Single finger pan while zoomed in
                            val current = pressed[0].position
                            val pan = current - prevCentroid
                            zoomOffsetX += pan.x
                            zoomOffsetY += pan.y
                            prevCentroid = current
                            event.changes.forEach { it.consume() }
                        } else {
                            // Single finger at zoom 1× — DON'T consume, let
                            // HorizontalPager handle horizontal swiping
                            prevCentroid = pressed[0].position
                        }
                    }
                }
            }
            .pointerInput(Unit) {
                detectTapGestures(
                    onDoubleTap = {
                        if (zoomScale > 1f) {
                            zoomScale = 1f
                            zoomOffsetX = 0f
                            zoomOffsetY = 0f
                        } else {
                            zoomScale = 2f
                        }
                    }
                )
            },
        contentAlignment = Alignment.Center
    ) {
        // Compute crop zoom modifier — disabled in edit mode so overlay aligns correctly
        val containerW = constraints.maxWidth.toFloat()
        val containerH = constraints.maxHeight.toFloat()

        // Use dynamically loaded size if available, otherwise fall back to DB values
        val baseW = if (intrinsicWidth > 0f) intrinsicWidth else (photo.width ?: 0).toFloat()
        val baseH = if (intrinsicHeight > 0f) intrinsicHeight else (photo.height ?: 0).toFloat()

        val cropModifier = if (
            !editMode && cropInfo != null && baseW > 0 && baseH > 0 &&
            containerW > 0 && containerH > 0
        ) {
            // When rotated 90/270, the image's effective dimensions are swapped
            val rot = cropInfo.rotate % 360
            val isSwapped = rot == 90 || rot == 270 || rot == -90 || rot == -270
            val effW = if (isSwapped) baseH else baseW
            val effH = if (isSwapped) baseW else baseH
            val imgAspect = effW / effH
            val containerAspect = containerW / containerH
            val rendW: Float
            val rendH: Float
            if (imgAspect > containerAspect) {
                rendW = containerW; rendH = containerW / imgAspect
            } else {
                rendH = containerH; rendW = containerH * imgAspect
            }
            val letterboxX = (containerW - rendW) / 2f
            val letterboxY = (containerH - rendH) / 2f
            val cx = cropInfo.x + cropInfo.width / 2f
            val cy = cropInfo.y + cropInfo.height / 2f
            val containerCX = (letterboxX + cx * rendW) / containerW
            val containerCY = (letterboxY + cy * rendH) / containerH
            val cropPixW = cropInfo.width * rendW
            val cropPixH = cropInfo.height * rendH
            val scale = minOf(containerW / cropPixW, containerH / cropPixH)
            val tx = containerW * (0.5f - containerCX)
            val ty = containerH * (0.5f - containerCY)

            // Extra scale needed to shrink the original (un-rotated) image so
            // that after rotation it fits within the container
            val rotScale = if (isSwapped && baseW > 0 && baseH > 0) {
                val origAspect = baseW / baseH
                val origRendW: Float; val origRendH: Float
                if (origAspect > containerAspect) {
                    origRendW = containerW; origRendH = containerW / origAspect
                } else {
                    origRendH = containerH; origRendW = containerH * origAspect
                }
                // After rotating the original rendered rect, its bounding box is swapped
                val rotatedBoundsW = origRendH
                val rotatedBoundsH = origRendW
                minOf(containerW / rotatedBoundsW, containerH / rotatedBoundsH)
            } else 1f

            Modifier.graphicsLayer {
                scaleX = scale * rotScale
                scaleY = scale * rotScale
                transformOrigin = TransformOrigin(containerCX, containerCY)
                translationX = tx
                translationY = ty
                rotationZ = cropInfo.rotate.toFloat()
            }
        } else if (!editMode && rotationDegrees != 0f) {
            // No crop but has saved rotation — scale so rotated image fits container
            val rot = rotationDegrees.toInt() % 360
            val isSwapped = rot == 90 || rot == 270 || rot == -90 || rot == -270
            if (isSwapped && baseW > 0 && baseH > 0 && containerW > 0 && containerH > 0) {
                val origAspect = baseW / baseH
                val containerAspect = containerW / containerH
                val origRendW: Float; val origRendH: Float
                if (origAspect > containerAspect) {
                    origRendW = containerW; origRendH = containerW / origAspect
                } else {
                    origRendH = containerH; origRendW = containerH * origAspect
                }
                val rotScale = minOf(containerW / origRendH, containerH / origRendW)
                Modifier.graphicsLayer {
                    scaleX = rotScale; scaleY = rotScale
                    rotationZ = rotationDegrees
                }
            } else {
                Modifier.graphicsLayer { rotationZ = rotationDegrees }
            }
        } else Modifier

        // Live rotation modifier for edit mode preview — includes scale so
        // a 90°/270° rotated image fits within the container bounds
        val editRotationModifier = if (editMode && rotationDegrees != 0f) {
            val rot = rotationDegrees.toInt() % 360
            val isSwapped = rot == 90 || rot == 270 || rot == -90 || rot == -270
            if (isSwapped && baseW > 0 && baseH > 0 && containerW > 0 && containerH > 0) {
                val origAspect = baseW / baseH
                val containerAspect = containerW / containerH
                val origRendW: Float; val origRendH: Float
                if (origAspect > containerAspect) {
                    origRendW = containerW; origRendH = containerW / origAspect
                } else {
                    origRendH = containerH; origRendW = containerH * origAspect
                }
                // Scale so the rotated bounding box (swapped W↔H) fits the container
                val rotScale = minOf(containerW / origRendH, containerH / origRendW)
                Modifier.graphicsLayer {
                    scaleX = rotScale; scaleY = rotScale
                    rotationZ = rotationDegrees
                }
            } else {
                Modifier.graphicsLayer { rotationZ = rotationDegrees }
            }
        } else Modifier

        // Zoom modifier — applied on top of crop transform
        val zoomModifier = if (zoomScale > 1f) {
            Modifier.graphicsLayer {
                scaleX = zoomScale
                scaleY = zoomScale
                translationX = zoomOffsetX
                translationY = zoomOffsetY
            }
        } else Modifier

        // Combined: crop first, then edit rotation, then zoom
        val combinedModifier = cropModifier.then(editRotationModifier).then(zoomModifier)

        when {
            // Loading encrypted content
            decryptLoading -> {
                CircularProgressIndicator(color = Color.White)
            }

            decryptError != null -> {
                Text(decryptError ?: "Error", color = Color.White, modifier = Modifier.padding(16.dp))
            }

            // ── Video / Audio ───────────────────────────────────────────
            photo.mediaType == "video" || photo.mediaType == "audio" -> {
                // Encrypted mode: temp file written during LaunchedEffect above.
                // Local mode: direct content URI.
                // All video playback now uses local files:
                //   • Encrypted mode: decrypted to temp file via LaunchedEffect above
                //   • Local mode: original file path
                val mediaUri = when {
                    hasLocalPath -> Uri.parse(photo.localPath)
                    tempMediaUri != null -> tempMediaUri
                    else -> null  // Still downloading — spinner shown by decryptLoading
                }
                if (mediaUri != null && sharedPlayer != null && onVideoUriReady != null) {
                    VideoPlayerPage(
                        uri = mediaUri,
                        sharedPlayer = sharedPlayer,
                        activeVideoUri = activeVideoUri,
                        isActivePage = isActivePage,
                        filename = photo.filename,
                        onVideoUriReady = onVideoUriReady,
                        onDurationKnown = onDurationKnown,
                        trimStart = trimStart,
                        trimEnd = trimEnd,
                        editMode = editMode,
                        editBrightness = editBrightness,
                        editRotation = editRotation,
                        savedBrightness = cropInfo?.brightness ?: 0f,
                        savedRotation = cropInfo?.rotate ?: 0,
                        photoWidth = if (intrinsicWidth > 0f) intrinsicWidth.toInt() else photo.width,
                        photoHeight = if (intrinsicHeight > 0f) intrinsicHeight.toInt() else photo.height,
                        playerError = playerError,
                        onMediaSizeLoaded = onMediaSizeLoaded
                    )
                } else if (mediaUri == null) {
                    Text("Media not available", color = Color.White)
                }
            }

            // ── Photo / GIF ────────────────────────────────────────────
            else -> {
                // Track image load errors for graceful fallback
                var imageError by remember(photo.localId) { mutableStateOf<String?>(null) }

                when {
                    imageError != null -> {
                        Column(
                            horizontalAlignment = Alignment.CenterHorizontally,
                            modifier = Modifier.padding(32.dp)
                        ) {
                            Text(
                                "Unable to display this format",
                                color = Color.White,
                                fontSize = 14.sp
                            )
                            Spacer(Modifier.height(4.dp))
                            Text(
                                photo.filename,
                                color = Color.Gray,
                                fontSize = 12.sp
                            )
                        }
                    }

                    // Local file — use Coil with content URI for memory-safe loading
                    hasLocalPath -> {
                        AsyncImage(
                            model = ImageRequest.Builder(context)
                                .data(Uri.parse(photo.localPath))
                                .crossfade(true)
                                .build(),
                            contentDescription = photo.filename,
                            modifier = Modifier.fillMaxSize().then(combinedModifier),
                            contentScale = ContentScale.Fit,
                            colorFilter = brightnessFilter,
                            onState = { state ->
                                if (state is AsyncImagePainter.State.Success) {
                                    val size = state.painter.intrinsicSize
                                    if (size.width > 0 && size.height > 0) {
                                        onMediaSizeLoaded?.invoke(size.width, size.height)
                                    }
                                }
                                if (state is AsyncImagePainter.State.Error) {
                                    Log.w("PhotoViewer", "Coil failed for local ${photo.filename}: ${state.result.throwable.message}")
                                    imageError = "Cannot decode ${photo.filename.substringAfterLast('.').uppercase()} format"
                                }
                            }
                        )
                    }

                    // Decrypted blob — use Coil with ByteArray for memory-safe decoding
                    // Coil handles GIF (via GifDecoder), SVG (via SvgDecoder), and
                    // standard formats while managing memory/downsampling automatically.
                    // For SVG files, write to a temp file so Coil's SvgDecoder can
                    // reliably detect the format via content sniffing.
                    decryptedData != null -> {
                        val bytes = decryptedData!!  // local copy for smart-cast safety
                        val isSvg = photo.mimeType.equals("image/svg+xml", ignoreCase = true)
                                || photo.filename.endsWith(".svg", ignoreCase = true)
                        val imageData: Any = if (isSvg) {
                            // Write to temp file — SvgDecoder needs reliable content sniffing
                            val svgFile = java.io.File(context.cacheDir, "svg_preview_${photo.localId}.svg")
                            if (!svgFile.exists() || svgFile.length() != bytes.size.toLong()) {
                                svgFile.writeBytes(bytes)
                            }
                            svgFile
                        } else {
                            bytes
                        }
                        AsyncImage(
                            model = ImageRequest.Builder(context)
                                .data(imageData)
                                .crossfade(true)
                                .build(),
                            contentDescription = photo.filename,
                            modifier = Modifier.fillMaxSize().then(combinedModifier),
                            contentScale = ContentScale.Fit,
                            colorFilter = brightnessFilter,
                            onState = { state ->
                                if (state is AsyncImagePainter.State.Success) {
                                    val size = state.painter.intrinsicSize
                                    if (size.width > 0 && size.height > 0) {
                                        onMediaSizeLoaded?.invoke(size.width, size.height)
                                    }
                                }
                                if (state is AsyncImagePainter.State.Error) {
                                    Log.w("PhotoViewer", "Coil failed for decrypted ${photo.filename}: ${state.result.throwable.message}")
                                    imageError = "Cannot display this format"
                                }
                            }
                        )
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Video/Audio page — renders the shared ExoPlayer's PlayerView when this
// is the active page.  No per-page ExoPlayer creation; the single shared
// instance is owned by PhotoViewerScreen.
// ---------------------------------------------------------------------------

@androidx.annotation.OptIn(UnstableApi::class)
@Composable
internal fun VideoPlayerPage(
    uri: Uri,
    sharedPlayer: ExoPlayer,
    activeVideoUri: Uri?,
    isActivePage: Boolean,
    filename: String,
    onVideoUriReady: (Uri, String) -> Unit,
    onDurationKnown: ((Float) -> Unit)? = null,
    // Trim boundaries (seconds) — applied during playback
    trimStart: Float = 0f,
    trimEnd: Float = 0f,
    editMode: Boolean = false,
    editBrightness: Float = 0f,
    editRotation: Int = 0,
    savedBrightness: Float = 0f,
    savedRotation: Int = 0,
    photoWidth: Int = 0,
    photoHeight: Int = 0,
    playerError: String?,
    onMediaSizeLoaded: ((Float, Float) -> Unit)? = null
) {
    // When this page becomes active, notify the screen to load our URI
    // into the shared player.
    LaunchedEffect(isActivePage, uri) {
        if (isActivePage) {
            onVideoUriReady(uri, filename)
        }
    }

    // Report duration when the player reports it
    LaunchedEffect(isActivePage, uri) {
        if (!isActivePage) return@LaunchedEffect
        val listener = object : androidx.media3.common.Player.Listener {
            override fun onPlaybackStateChanged(state: Int) {
                if (state == androidx.media3.common.Player.STATE_READY) {
                    val dur = sharedPlayer.duration
                    if (dur > 0 && onDurationKnown != null) {
                        onDurationKnown(dur / 1000f)
                    }
                }
            }
            override fun onVideoSizeChanged(videoSize: androidx.media3.common.VideoSize) {
                if (videoSize.width > 0 && videoSize.height > 0) {
                    // videoSize doesn't account for display rotation directly in all players,
                    // but ExoPlayer often returns the rotated intrinsic dimensions or provides unappliedRotationDegrees.
                    // For now, return what ExoPlayer says is the video size.
                    onMediaSizeLoaded?.invoke(videoSize.width.toFloat(), videoSize.height.toFloat())
                }
            }
        }
        sharedPlayer.addListener(listener)
        // Also check current state in case already ready
        if (sharedPlayer.playbackState == androidx.media3.common.Player.STATE_READY) {
            if (sharedPlayer.duration > 0 && onDurationKnown != null) {
                onDurationKnown(sharedPlayer.duration / 1000f)
            }
            if (sharedPlayer.videoSize.width > 0 && sharedPlayer.videoSize.height > 0) {
                onMediaSizeLoaded?.invoke(sharedPlayer.videoSize.width.toFloat(), sharedPlayer.videoSize.height.toFloat())
            }
        }
        kotlinx.coroutines.suspendCancellableCoroutine<Unit> { cont ->
            cont.invokeOnCancellation { sharedPlayer.removeListener(listener) }
        }
    }

    // Enforce trim boundaries during playback — seek to trimStart when
    // playback begins, pause when trimEnd is reached.
    LaunchedEffect(isActivePage, trimStart, trimEnd) {
        if (!isActivePage) return@LaunchedEffect
        val listener = object : androidx.media3.common.Player.Listener {
            override fun onPlaybackStateChanged(state: Int) {
                if (state == androidx.media3.common.Player.STATE_READY) {
                    // Seek to trimStart if set and we're before it
                    if (trimStart > 0.01f) {
                        val currentMs = sharedPlayer.currentPosition
                        if (currentMs < (trimStart * 1000).toLong() - 500) {
                            sharedPlayer.seekTo((trimStart * 1000).toLong())
                        }
                    }
                }
            }

            override fun onEvents(player: androidx.media3.common.Player, events: androidx.media3.common.Player.Events) {
                // Check trim end boundary
                if (trimEnd > 0.01f) {
                    val currentMs = sharedPlayer.currentPosition
                    if (currentMs >= (trimEnd * 1000).toLong()) {
                        sharedPlayer.playWhenReady = false
                        sharedPlayer.seekTo((trimEnd * 1000).toLong())
                    }
                }
            }
        }
        sharedPlayer.addListener(listener)
        kotlinx.coroutines.suspendCancellableCoroutine<Unit> { cont ->
            cont.invokeOnCancellation { sharedPlayer.removeListener(listener) }
        }
    }

    // Pause the shared player when swiping away from a video page.
    // (The next video page will call onVideoUriReady which re-prepares.)
    LaunchedEffect(isActivePage) {
        if (!isActivePage && activeVideoUri == uri) {
            sharedPlayer.playWhenReady = false
        }
    }

    if (playerError != null && isActivePage) {
        // Fallback UI for unsupported formats
        Box(
            modifier = Modifier.fillMaxSize(),
            contentAlignment = Alignment.Center
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    "Unable to play this video format",
                    color = Color.White,
                    fontSize = 14.sp
                )
                Spacer(Modifier.height(4.dp))
                Text(
                    filename,
                    color = Color.Gray,
                    fontSize = 12.sp
                )
            }
        }
    } else if (isActivePage && activeVideoUri == uri) {
        // This is the active video page and the shared player has our URI loaded.
        // Apply brightness and rotation transforms.
        val activeBrightness = if (editMode) editBrightness else savedBrightness
        val activeRotation = if (editMode) editRotation else savedRotation

        // For 90/270 rotation, scale down so the rotated video fits the container.
        BoxWithConstraints(modifier = Modifier.fillMaxSize()) {
            val containerW = constraints.maxWidth.toFloat()
            val containerH = constraints.maxHeight.toFloat()
            val rot = activeRotation % 360
            val isSwapped = rot == 90 || rot == 270 || rot == -90 || rot == -270
            val videoRotationModifier = if (activeRotation != 0) {
                if (isSwapped && photoWidth > 0 && photoHeight > 0 && containerW > 0 && containerH > 0) {
                    // Known media dimensions — precise scaling so the rotated video fits
                    val origAspect = photoWidth.toFloat() / photoHeight.toFloat()
                    val containerAspect = containerW / containerH
                    val origRendW: Float
                    val origRendH: Float
                    if (origAspect > containerAspect) {
                        origRendW = containerW; origRendH = containerW / origAspect
                    } else {
                        origRendH = containerH; origRendW = containerH * origAspect
                    }
                    val rotScale = minOf(containerW / origRendH, containerH / origRendW)
                    Modifier.graphicsLayer {
                        scaleX = rotScale; scaleY = rotScale
                        rotationZ = activeRotation.toFloat()
                    }
                } else if (isSwapped && containerW > 0 && containerH > 0) {
                    // Unknown media dimensions — use container-based scale to prevent overflow
                    val rotScale = minOf(containerW / containerH, containerH / containerW)
                    Modifier.graphicsLayer {
                        scaleX = rotScale; scaleY = rotScale
                        rotationZ = activeRotation.toFloat()
                    }
                } else {
                    Modifier.graphicsLayer { rotationZ = activeRotation.toFloat() }
                }
            } else Modifier

            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .then(videoRotationModifier)
            ) {
                AndroidView(
                factory = { ctx ->
                    PlayerView(ctx).apply {
                        this.player = sharedPlayer
                        useController = false
                    }
                },
                update = { playerView ->
                    playerView.player = sharedPlayer
                },
                modifier = Modifier.fillMaxSize()
            )
            // Brightness overlay: positive = brighten (white overlay),
            // negative = darken (black overlay). Matches CSS brightness() filter.
            if (activeBrightness != 0f) {
                val overlayColor = if (activeBrightness > 0f) {
                    Color.White.copy(alpha = (activeBrightness / 200f).coerceIn(0f, 0.5f))
                } else {
                    Color.Black.copy(alpha = (-activeBrightness / 150f).coerceIn(0f, 0.7f))
                }
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .background(overlayColor)
                )
            }
        }

            // Tap catcher — toggles custom controls visibility.
            // Sits between the video and the controls overlay so taps on
            // the video area trigger show/hide while taps on control widgets
            // are handled by those widgets directly (they stack above this).
            var showControls by remember { mutableStateOf(true) }

            // Auto-hide controls after 3 seconds when video is playing
            val playerIsPlaying = remember { mutableStateOf(sharedPlayer.isPlaying) }
            DisposableEffect(sharedPlayer) {
                val listener = object : Player.Listener {
                    override fun onIsPlayingChanged(playing: Boolean) {
                        playerIsPlaying.value = playing
                    }
                }
                sharedPlayer.addListener(listener)
                onDispose { sharedPlayer.removeListener(listener) }
            }
            LaunchedEffect(showControls, playerIsPlaying.value) {
                if (showControls && playerIsPlaying.value) {
                    delay(3000)
                    showControls = false
                }
            }

            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null
                    ) { showControls = !showControls }
            )

            // Custom controls overlay — NOT rotated
            VideoControlsOverlay(
                player = sharedPlayer,
                visible = showControls,
                modifier = Modifier.align(Alignment.BottomCenter)
            )
        }
    } else {
        // Not active or player showing a different video — black placeholder
        Box(
            modifier = Modifier.fillMaxSize().background(Color.Black),
            contentAlignment = Alignment.Center
        ) {
            // Intentionally blank
        }
    }
}

// ---------------------------------------------------------------------------
// Custom video controls overlay — always renders upright (outside rotation)
// ---------------------------------------------------------------------------

@Composable
internal fun VideoControlsOverlay(
    player: ExoPlayer,
    visible: Boolean,
    modifier: Modifier = Modifier
) {
    // ── Reactive player state ──────────────────────────────────────────────
    var isPlaying by remember { mutableStateOf(player.isPlaying) }
    var currentPosition by remember { mutableLongStateOf(player.currentPosition) }
    var duration by remember { mutableLongStateOf(player.duration.coerceAtLeast(0L)) }
    var isMuted by remember { mutableStateOf(player.volume == 0f) }
    var isSeeking by remember { mutableStateOf(false) }
    var seekFraction by remember { mutableFloatStateOf(0f) }

    DisposableEffect(player) {
        val listener = object : Player.Listener {
            override fun onIsPlayingChanged(playing: Boolean) { isPlaying = playing }
            override fun onPlaybackStateChanged(state: Int) {
                val d = player.duration
                if (d > 0) duration = d
            }
        }
        player.addListener(listener)
        isPlaying = player.isPlaying
        isMuted = player.volume == 0f
        val d = player.duration; if (d > 0) duration = d
        onDispose { player.removeListener(listener) }
    }

    // Poll position for smooth seek bar (every 250ms)
    LaunchedEffect(player) {
        while (true) {
            if (!isSeeking) currentPosition = player.currentPosition
            delay(250)
        }
    }

    val progress = if (duration > 0) currentPosition.toFloat() / duration.toFloat() else 0f

    AnimatedVisibility(
        visible = visible,
        enter = fadeIn(),
        exit = fadeOut(),
        modifier = modifier.fillMaxWidth()
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .background(
                    Brush.verticalGradient(
                        colors = listOf(Color.Transparent, Color.Black.copy(alpha = 0.8f))
                    )
                )
                // Consume taps within the controls area so they don't toggle visibility
                .clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null
                ) { /* no-op */ }
                .padding(horizontal = 16.dp)
                .padding(top = 32.dp, bottom = 8.dp)
        ) {
            // Seek bar
            Slider(
                value = if (isSeeking) seekFraction else progress,
                onValueChange = {
                    isSeeking = true
                    seekFraction = it
                },
                onValueChangeFinished = {
                    player.seekTo((seekFraction * duration).toLong())
                    isSeeking = false
                },
                colors = SliderDefaults.colors(
                    thumbColor = Color.White,
                    activeTrackColor = Color(0xFF3B82F6),
                    inactiveTrackColor = Color.White.copy(alpha = 0.2f)
                ),
                modifier = Modifier.fillMaxWidth()
            )

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                // Play / Pause
                IconButton(onClick = {
                    if (player.isPlaying) player.pause() else player.play()
                }) {
                    Icon(
                        imageVector = if (isPlaying) Icons.Filled.Pause else Icons.Filled.PlayArrow,
                        contentDescription = if (isPlaying) "Pause" else "Play",
                        tint = Color.White,
                        modifier = Modifier.size(28.dp)
                    )
                }

                // Time
                Text(
                    text = "${formatPlayerTime(currentPosition)} / ${formatPlayerTime(duration)}",
                    color = Color.White.copy(alpha = 0.8f),
                    fontSize = 12.sp,
                    fontFamily = FontFamily.Monospace
                )

                Spacer(Modifier.weight(1f))

                // Mute / unmute
                IconButton(onClick = {
                    if (player.volume == 0f) { player.volume = 1f; isMuted = false }
                    else { player.volume = 0f; isMuted = true }
                }) {
                    Icon(
                        imageVector = if (isMuted) Icons.AutoMirrored.Filled.VolumeOff else Icons.AutoMirrored.Filled.VolumeUp,
                        contentDescription = if (isMuted) "Unmute" else "Mute",
                        tint = Color.White.copy(alpha = 0.7f),
                        modifier = Modifier.size(22.dp)
                    )
                }
            }
        }
    }
}

private fun formatPlayerTime(ms: Long): String {
    if (ms <= 0) return "0:00"
    val totalSec = ms / 1000
    val h = totalSec / 3600
    val m = (totalSec % 3600) / 60
    val s = totalSec % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, s) else "%d:%02d".format(m, s)
}

// ── Info detail helpers ──────────────────────────────────────────────────────

@Composable
internal fun InfoDetailRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(label, color = Color.Gray, fontSize = 13.sp)
        Text(value, color = Color.White, fontSize = 13.sp)
    }
}

internal fun formatInfoBytes(bytes: Long): String {
    if (bytes <= 0) return "0 B"
    val units = arrayOf("B", "KB", "MB", "GB")
    val i = (Math.log10(bytes.toDouble()) / Math.log10(1024.0)).toInt().coerceAtMost(units.lastIndex)
    return "%.1f %s".format(bytes / Math.pow(1024.0, i.toDouble()), units[i])
}
