/**
 * Overlays that handle "special" photo types in [PhotoViewerScreen]:
 *   - Panorama   (photo_subtype == "panorama"):       horizontal-pan flat viewer
 *   - 360 Sphere (photo_subtype == "equirectangular"): horizontal-pan flat viewer
 *                (true WebGL sphere is web-only; the flat pan mirrors the web
 *                fallback behaviour)
 *   - Motion     (motion_video_blob_id != null):      LIVE video overlay played
 *                via Media3 ExoPlayer on top of the still image
 *
 * Mirrors the web's PanoramaViewer + MotionVideoOverlay components so both
 * platforms expose the same interactive features.
 */
package com.simplephotos.ui.screens.viewer

import android.net.Uri
import android.util.Log
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.media3.common.MediaItem
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import androidx.compose.ui.viewinterop.AndroidView
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import coil.request.ImageRequest
import com.simplephotos.data.local.entities.PhotoEntity
import okhttp3.OkHttpClient
import okhttp3.Request

// ─── Panorama / 360° flat-pan overlay ───────────────────────────────────────

/**
 * Wraps an image source in a horizontally-scrollable viewport. Tapping the
 * mode pill at the bottom toggles between "Full" (object-fit) and "Live"
 * (rendered at native aspect, draggable horizontally).
 *
 * This is a lightweight equivalent of the web's [PanoramaViewer] flat mode.
 */
@Composable
fun PanoramaOverlay(
    imageData: Any?,
    intrinsicWidth: Float,
    intrinsicHeight: Float,
    is360: Boolean,
    contentDescription: String,
) {
    if (imageData == null) return
    var liveMode by remember { mutableStateOf(false) }

    if (!liveMode) {
        // Mode toggle pill is rendered at the bottom; underlying image is
        // drawn by the parent's standard photo branch, so we only emit the
        // toggle button here.
        ModeTogglePill(
            label = if (is360) "360°" else "PANO",
            sub = "Live View",
            onClick = { liveMode = true }
        )
        return
    }

    // Live mode: paint our own pannable image on top of everything else.
    val context = LocalContext.current
    val density = LocalDensity.current
    var containerSize by remember { mutableStateOf(Size.Zero) }
    var panX by remember { mutableStateOf(0f) }

    val aspect = if (intrinsicWidth > 0 && intrinsicHeight > 0)
        intrinsicWidth / intrinsicHeight else 2f
    val renderedHeightPx = containerSize.height
    val renderedWidthPx = renderedHeightPx * aspect
    val maxPan = (renderedWidthPx - containerSize.width).coerceAtLeast(0f)

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black)
            .clipToBounds()
            .pointerInput(Unit) {
                // Consume horizontal drag in the INITIAL pass so the parent
                // HorizontalPager (in PhotoViewerScreen) does not see it and
                // try to flip pages — which would prevent panning the pano.
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent(PointerEventPass.Initial)
                        var dx = 0f
                        event.changes.forEach { change ->
                            if (change.pressed) {
                                dx += change.positionChange().x
                                change.consume()
                            }
                        }
                        if (dx != 0f) {
                            panX = (panX - dx).coerceIn(0f, maxPan)
                        }
                    }
                }
            }
            .onSizeChanged { containerSize = Size(it.width.toFloat(), it.height.toFloat()) }
    ) {
        AsyncImage(
            model = ImageRequest.Builder(context).data(imageData).crossfade(true).build(),
            contentDescription = contentDescription,
            contentScale = ContentScale.FillHeight,
            modifier = Modifier
                .fillMaxHeight()
                .graphicsLayer { translationX = -panX }
        )

        // Pan position indicator
        if (maxPan > 0f) {
            Box(
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 100.dp)
                    .height(3.dp)
                    .width(120.dp)
                    .clip(RoundedCornerShape(2.dp))
                    .background(Color.White.copy(alpha = 0.25f))
            ) {
                val frac = if (renderedWidthPx > 0f)
                    (panX / renderedWidthPx).coerceIn(0f, 1f) else 0f
                val barWidthFrac = (containerSize.width / renderedWidthPx).coerceIn(0f, 1f)
                Box(
                    modifier = Modifier
                        .fillMaxHeight()
                        .fillMaxWidth(barWidthFrac)
                        .graphicsLayer { translationX = (frac * 120 * density.density) }
                        .background(Color.White)
                )
            }
        }

        ModeTogglePill(
            label = if (is360) "360°" else "PANO",
            sub = "Full View",
            onClick = { liveMode = false }
        )
    }
}

@Composable
private fun ModeTogglePill(label: String, sub: String, onClick: () -> Unit) {
    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.BottomCenter
    ) {
        androidx.compose.material3.Surface(
            modifier = Modifier
                .padding(bottom = 80.dp)
                .clip(CircleShape)
                .clickable(onClick = onClick),
            color = Color.Black.copy(alpha = 0.6f),
            shape = CircleShape
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp)
            ) {
                Text(label, color = Color.White, fontWeight = FontWeight.Bold, fontSize = 11.sp)
                Spacer(Modifier.width(8.dp))
                Text(sub, color = Color.White, fontSize = 12.sp)
            }
        }
    }
}

// ─── Motion (LIVE) photo overlay ────────────────────────────────────────────

/**
 * Auto-plays the embedded motion-photo video on top of the still image.
 * Mirrors the web's MotionVideoOverlay.
 */
@OptIn(UnstableApi::class)
@Composable
fun MotionPhotoOverlay(
    photo: PhotoEntity,
    serverBaseUrl: String,
    okHttpClient: OkHttpClient,
) {
    val motionBlobId = photo.motionVideoBlobId ?: return
    val context = LocalContext.current
    var playing by remember(photo.localId) { mutableStateOf(true) }
    var loading by remember(photo.localId) { mutableStateOf(true) }
    var localUri by remember(photo.localId) { mutableStateOf<Uri?>(null) }
    var error by remember(photo.localId) { mutableStateOf(false) }

    // Download motion video to a temp file (private, so ExoPlayer can read it).
    LaunchedEffect(photo.localId, motionBlobId) {
        try {
            val url = "$serverBaseUrl/api/blobs/$motionBlobId/download"
            val req = Request.Builder().url(url).build()
            okHttpClient.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) {
                    error = true; loading = false
                    return@use
                }
                val body = resp.body ?: run { error = true; loading = false; return@use }
                val tmp = java.io.File(context.cacheDir, "motion_${photo.localId}.mp4")
                tmp.outputStream().use { out -> body.byteStream().copyTo(out) }
                localUri = Uri.fromFile(tmp)
                loading = false
            }
        } catch (e: Exception) {
            Log.w("MotionPhotoOverlay", "download failed: ${e.message}")
            error = true; loading = false
        }
    }

    if (error) return

    val player = remember(localUri) {
        if (localUri == null) null else ExoPlayer.Builder(context).build().apply {
            setMediaItem(MediaItem.fromUri(localUri!!))
            repeatMode = androidx.media3.common.Player.REPEAT_MODE_ALL
            volume = 0f
            prepare()
            playWhenReady = true
        }
    }
    DisposableEffect(player) { onDispose { player?.release() } }

    LaunchedEffect(playing, player) {
        player?.playWhenReady = playing
    }

    Box(modifier = Modifier.fillMaxSize()) {
        if (player != null && playing) {
            AndroidView(
                modifier = Modifier.fillMaxSize(),
                factory = { ctx ->
                    PlayerView(ctx).apply {
                        useController = false
                        this.player = player
                    }
                }
            )
        }

        if (loading) {
            Box(
                modifier = Modifier
                    .align(Alignment.TopCenter)
                    .padding(top = 64.dp)
            ) {
                CircularProgressIndicator(strokeWidth = 2.dp, modifier = Modifier.size(20.dp), color = Color.White)
            }
        }

        // LIVE toggle button
        Box(
            modifier = Modifier.fillMaxSize(),
            contentAlignment = Alignment.BottomCenter
        ) {
            androidx.compose.material3.Surface(
                modifier = Modifier
                    .padding(bottom = 80.dp)
                    .clip(CircleShape)
                    .clickable { playing = !playing },
                color = if (playing) Color.White else Color.Black.copy(alpha = 0.6f),
                shape = CircleShape
            ) {
                Text(
                    text = if (playing) "LIVE \u25CF" else "LIVE \u25CB",
                    color = if (playing) Color.Black else Color.White,
                    fontWeight = FontWeight.Bold,
                    fontSize = 12.sp,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp)
                )
            }
        }
    }
}

// ─── helper: end of file ───────────────────────────────────────────────────
