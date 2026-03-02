package com.simplephotos.ui.screens.viewer

import android.graphics.BitmapFactory
import android.net.Uri
import androidx.compose.foundation.Image
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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.MediaItem
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.PlayerView
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.PhotoEntity
import okhttp3.OkHttpClient
import org.json.JSONObject

// Parsed crop metadata for zoom/brightness transforms
internal data class CropInfo(
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val brightness: Float
)

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
    editMode: Boolean = false,
    editBrightness: Float = 0f
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
                val mediaUri = when {
                    isPlainMode -> Uri.parse("$serverBaseUrl/api/photos/${photo.serverPhotoId}/file")
                    hasLocalPath -> Uri.parse(photo.localPath)
                    decryptedData != null -> {
                        val ext = if (photo.mediaType == "audio") ".mp3" else ".mp4"
                        val tempFile = remember(photo.localId, decryptedData) {
                            java.io.File.createTempFile("media_", ext, context.cacheDir).apply {
                                writeBytes(decryptedData!!)
                                deleteOnExit()
                            }
                        }
                        Uri.fromFile(tempFile)
                    }
                    else -> null
                }
                if (mediaUri != null) {
                    VideoPlayer(uri = mediaUri, okHttpClient = okHttpClient)
                } else {
                    Text("Media not available", color = Color.White)
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
                            modifier = Modifier.fillMaxSize().then(combinedModifier),
                            contentScale = ContentScale.Fit,
                            colorFilter = brightnessFilter
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
                                modifier = Modifier.fillMaxSize().then(combinedModifier),
                                contentScale = ContentScale.Fit,
                                colorFilter = brightnessFilter
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
                                modifier = Modifier.fillMaxSize().then(combinedModifier),
                                contentScale = ContentScale.Fit,
                                colorFilter = brightnessFilter
                            )
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Video/Audio player composable (ExoPlayer + OkHttp for auth)
// ---------------------------------------------------------------------------

@androidx.annotation.OptIn(UnstableApi::class)
@Composable
internal fun VideoPlayer(uri: Uri, okHttpClient: OkHttpClient) {
    val context = LocalContext.current
    val player = remember {
        val dataSourceFactory = OkHttpDataSource.Factory(okHttpClient)
        val mediaSourceFactory = DefaultMediaSourceFactory(dataSourceFactory)
        ExoPlayer.Builder(context)
            .setMediaSourceFactory(mediaSourceFactory)
            .build()
            .apply {
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
