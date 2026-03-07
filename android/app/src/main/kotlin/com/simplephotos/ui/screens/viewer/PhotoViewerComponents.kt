package com.simplephotos.ui.screens.viewer

import android.net.Uri
import android.util.Log
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.PhotoEntity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject

// Parsed crop metadata for zoom/brightness transforms
internal data class CropInfo(
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val brightness: Float
)

/**
 * Mirrors the server's `needs_web_preview()` logic. Returns the target
 * extension if this filename's format requires conversion for native
 * Android playback, or null if Android can handle it directly.
 */
internal fun needsWebPreview(filename: String): String? {
    val ext = filename.substringAfterLast('.', "").lowercase()
    return when (ext) {
        // Images not decodable by Android BitmapFactory / Coil natively
        "heic", "heif", "tiff", "tif", "hdr", "cr2", "cur", "cursor",
        "dng", "nef", "arw", "raw" -> "jpg"
        // ICO — not natively supported by BitmapFactory
        "ico" -> "png"
        // SVG — Coil can handle if coil-svg is present, but the /web
        // endpoint provides a rasterised PNG for consistent rendering
        "svg" -> "png"
        // Video containers not reliably playable by ExoPlayer
        "mkv", "avi", "wmv", "asf", "h264",
        "mpg", "mpeg", "3gp", "mov", "m4v" -> "mp4"
        // Audio formats not natively supported
        "wma", "aiff", "aif" -> "mp3"
        else -> null
    }
}

// ---------------------------------------------------------------------------
// Per-page content — each page independently loads and renders its photo
// ---------------------------------------------------------------------------

@Composable
internal fun PhotoPageContent(
    photo: PhotoEntity,
    encryptionMode: String,
    serverBaseUrl: String,
    viewModel: PhotoViewerViewModel,
    okHttpClient: OkHttpClient,
    isActivePage: Boolean = true,
    editMode: Boolean = false,
    editBrightness: Float = 0f,
    // Shared ExoPlayer — owned by PhotoViewerScreen, one instance for all pages
    sharedPlayer: ExoPlayer? = null,
    activeVideoUri: Uri? = null,
    onVideoUriReady: ((Uri, String) -> Unit)? = null,
    playerError: String? = null,
    isConverting: Boolean = false
) {
    val context = LocalContext.current

    // Determine the content source for this photo
    val isPlainMode = encryptionMode == "plain" && photo.serverPhotoId != null
    val hasLocalPath = photo.localPath != null
    val hasEncryptedBlob = photo.serverBlobId != null

    // For encrypted mode, lazily download & decrypt.
    // Photos: store ByteArray for Coil.
    // Videos: write to temp file and store URI to avoid holding large blobs in memory.
    val isMedia = photo.mediaType == "video" || photo.mediaType == "audio"
    var decryptedData by remember(photo.localId) { mutableStateOf<ByteArray?>(null) }
    var tempMediaUri by remember(photo.localId) { mutableStateOf<Uri?>(null) }
    // Show loading spinner when media needs download (encrypted OR plain video)
    val needsMediaLoad = (!isPlainMode && !hasLocalPath && hasEncryptedBlob) ||
        (isPlainMode && isMedia && !hasLocalPath)
    var decryptLoading by remember(photo.localId) { mutableStateOf(needsMediaLoad) }
    var decryptError by remember(photo.localId) { mutableStateOf<String?>(null) }

    // For videos, gate on isActivePage so we don't download a 50 MB video
    // for the next page while the current page's ExoPlayer is still alive.
    // For photos, download eagerly so Coil can display them during swipe.
    val shouldDecrypt = !isPlainMode && !hasLocalPath && hasEncryptedBlob &&
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
                    else if (needsWebPreview(photo.filename) == "mp4") ".mp4"
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

    // ── Plain-mode video: stream-download to temp file ────────────────
    // Instead of letting ExoPlayer stream over HTTP (which pulls data
    // into the Java heap via OkHttp buffers), we download the video to
    // a temp file first using a tiny 8 KB streaming copy.  ExoPlayer
    // then reads from local storage with near-zero heap usage.
    // This is why the web browser doesn't OOM: it downloads, then plays.
    val shouldDownloadPlainVideo = isPlainMode && isMedia && !hasLocalPath && isActivePage
    LaunchedEffect(photo.localId, shouldDownloadPlainVideo) {
        if (shouldDownloadPlainVideo && tempMediaUri == null) {
            decryptLoading = true
            try {
                val url = "$serverBaseUrl/api/photos/${photo.serverPhotoId}/web"
                val ext = "." + photo.filename.substringAfterLast('.', "mp4").lowercase()
                val uri = withContext(Dispatchers.IO) {
                    val request = Request.Builder().url(url).build()
                    okHttpClient.newCall(request).execute().use { response ->
                        if (!response.isSuccessful) {
                            throw Exception("Server error: ${response.code}")
                        }
                        val tempFile = java.io.File.createTempFile("media_", ext, context.cacheDir)
                        // Stream directly to disk — only 8 KB in heap at a time.
                        // A 200 MB video file never touches the Java heap.
                        response.body?.byteStream()?.use { input ->
                            tempFile.outputStream().buffered().use { output ->
                                input.copyTo(output, bufferSize = 8192)
                            }
                        } ?: throw Exception("Empty response body")
                        Uri.fromFile(tempFile)
                    }
                }
                tempMediaUri = uri
            } catch (e: Throwable) {
                decryptError = e.message ?: "Failed to download video"
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

    // Parse crop metadata for zoom/brightness display
    val cropInfo = remember(photo.cropMetadata) {
        photo.cropMetadata?.let {
            try {
                val json = JSONObject(it)
                CropInfo(
                    x = json.optDouble("x", 0.0).toFloat(),
                    y = json.optDouble("y", 0.0).toFloat(),
                    width = json.optDouble("width", 1.0).toFloat(),
                    height = json.optDouble("height", 1.0).toFloat(),
                    brightness = json.optDouble("brightness", 0.0).toFloat()
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
        val cropModifier = if (
            !editMode && cropInfo != null && photo.width > 0 && photo.height > 0 &&
            containerW > 0 && containerH > 0
        ) {
            val imgAspect = photo.width.toFloat() / photo.height.toFloat()
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
            Modifier.graphicsLayer {
                scaleX = scale
                scaleY = scale
                transformOrigin = TransformOrigin(containerCX, containerCY)
                translationX = tx
                translationY = ty
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

        // Combined: crop first, then zoom
        val combinedModifier = cropModifier.then(zoomModifier)

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
                // Plain mode: /web endpoint serves FFmpeg-converted MP4.
                // Encrypted mode: temp file written during LaunchedEffect above.
                // Local mode: direct content URI.
                // All video playback now uses local files:
                //   • Plain mode: downloaded to temp file via LaunchedEffect above
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
                        playerError = playerError,
                        isConverting = isConverting
                    )
                } else if (mediaUri == null) {
                    Text("Media not available", color = Color.White)
                }
            }

            // ── Photo / GIF ────────────────────────────────────────────
            else -> {
                // Track image load errors for graceful fallback
                var imageError by remember(photo.localId) { mutableStateOf<String?>(null) }
                var isConverting by remember(photo.localId) { mutableStateOf(false) }

                when {
                    imageError != null -> {
                        Column(
                            horizontalAlignment = Alignment.CenterHorizontally,
                            modifier = Modifier.padding(32.dp)
                        ) {
                            if (isConverting) {
                                CircularProgressIndicator(color = Color.White)
                                Spacer(Modifier.height(16.dp))
                                Text(
                                    "Converting to compatible format…",
                                    color = Color.Gray,
                                    fontSize = 14.sp
                                )
                            } else {
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
                    }

                    // Plain mode: Coil loads via authenticated /web endpoint
                    // Server converts non-native formats (CR2, TIFF, SVG, ICO, etc.)
                    isPlainMode -> {
                        val webUrl = "$serverBaseUrl/api/photos/${photo.serverPhotoId}/web"
                        AsyncImage(
                            model = ImageRequest.Builder(context)
                                .data(webUrl)
                                .crossfade(true)
                                .build(),
                            contentDescription = photo.filename,
                            modifier = Modifier.fillMaxSize().then(combinedModifier),
                            contentScale = ContentScale.Fit,
                            colorFilter = brightnessFilter,
                            onState = { state ->
                                if (state is AsyncImagePainter.State.Error) {
                                    Log.w("PhotoViewer", "Coil failed for ${photo.filename}: ${state.result.throwable.message}")
                                    // If format needs conversion, server may still be converting (202)
                                    if (needsWebPreview(photo.filename) != null) {
                                        isConverting = true
                                    }
                                    imageError = state.result.throwable.message ?: "Failed to load"
                                }
                            }
                        )
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
                                if (state is AsyncImagePainter.State.Error) {
                                    Log.w("PhotoViewer", "Coil failed for local ${photo.filename}: ${state.result.throwable.message}")
                                    imageError = "Cannot decode ${photo.filename.substringAfterLast('.').uppercase()} format"
                                }
                            }
                        )
                    }

                    // Decrypted blob — use Coil with ByteArray for memory-safe decoding
                    // Coil handles GIF (via GifDecoder), SVG (via SvgDecoder), and
                    // standard formats while managing memory/downsampling automatically
                    decryptedData != null -> {
                        AsyncImage(
                            model = ImageRequest.Builder(context)
                                .data(decryptedData)
                                .crossfade(true)
                                .build(),
                            contentDescription = photo.filename,
                            modifier = Modifier.fillMaxSize().then(combinedModifier),
                            contentScale = ContentScale.Fit,
                            colorFilter = brightnessFilter,
                            onState = { state ->
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
    playerError: String?,
    isConverting: Boolean
) {
    // When this page becomes active, notify the screen to load our URI
    // into the shared player.
    LaunchedEffect(isActivePage, uri) {
        if (isActivePage) {
            onVideoUriReady(uri, filename)
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
        // Fallback UI for unsupported formats or conversion-in-progress
        Box(
            modifier = Modifier.fillMaxSize(),
            contentAlignment = Alignment.Center
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                if (isConverting) {
                    CircularProgressIndicator(color = Color.White)
                    Spacer(Modifier.height(16.dp))
                    Text(
                        "Converting to compatible format\u2026",
                        color = Color.Gray,
                        fontSize = 14.sp
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        "This may take a moment",
                        color = Color.Gray,
                        fontSize = 12.sp
                    )
                } else {
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
        }
    } else if (isActivePage && activeVideoUri == uri) {
        // This is the active video page and the shared player has our URI loaded.
        // Show the PlayerView. Default rendering uses SurfaceView which sends
        // decoded frames directly to the display compositor in native memory,
        // avoiding the Java-heap Bitmap copy that TextureView would require.
        AndroidView(
            factory = { ctx ->
                PlayerView(ctx).apply {
                    this.player = sharedPlayer
                    useController = true
                }
            },
            update = { playerView ->
                playerView.player = sharedPlayer
            },
            modifier = Modifier.fillMaxSize()
        )
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
