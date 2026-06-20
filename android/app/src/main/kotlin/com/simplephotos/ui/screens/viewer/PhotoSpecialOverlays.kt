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
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

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
    onLiveModeChange: (Boolean) -> Unit = {},
) {
    if (imageData == null) return
    var liveMode by remember { mutableStateOf(false) }

    // Notify caller (PhotoViewerScreen) so it can disable HorizontalPager
    // paging while the user is panning the panorama / 360 image.
    LaunchedEffect(liveMode) { onLiveModeChange(liveMode) }
    DisposableEffect(Unit) {
        onDispose { onLiveModeChange(false) }
    }

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

    // ── Live mode, equirectangular: real WebGL-style photo sphere ────────────
    // Mirrors the web's Sphere360Viewer (drag to look around, pinch to zoom)
    // instead of the old flat horizontal pan, which never gave a 360 feel.
    if (is360) {
        Sphere360Overlay(
            imageData = imageData,
            contentDescription = contentDescription,
            onExitToFull = { liveMode = false },
        )
        return
    }

    // ── Live mode, flat panorama: pan along the image's LONG axis ────────────
    // Mirrors the web PanoramaViewer flat mode: horizontal stitches pan
    // left/right; tall (vertical) panoramas pan up/down.  The previous
    // implementation only ever panned horizontally AND captured maxPan from the
    // first (zero-size) composition, so the pan range was pinned to 0 and the
    // image could not move at all.
    val context = LocalContext.current
    val density = LocalDensity.current
    var containerSize by remember { mutableStateOf(Size.Zero) }
    var pan by remember { mutableStateOf(0f) }

    val aspect = if (intrinsicWidth > 0 && intrinsicHeight > 0)
        intrinsicWidth / intrinsicHeight else 2f
    val horizontal = aspect >= 1f

    // Rendered dimensions: the short axis fills the viewport, the long axis
    // overflows and is translated.
    val renderedWidthPx = if (horizontal) containerSize.height * aspect else containerSize.width
    val renderedHeightPx = if (horizontal) containerSize.height else containerSize.width / aspect
    val viewportSpan = if (horizontal) containerSize.width else containerSize.height
    val renderedSpan = if (horizontal) renderedWidthPx else renderedHeightPx
    val maxPan = (renderedSpan - viewportSpan).coerceAtLeast(0f)

    // The gesture coroutine below is keyed on Unit so it is NOT restarted on
    // every size change. Read these through rememberUpdatedState so the running
    // loop always sees the CURRENT maxPan / axis instead of the stale
    // first-composition values (the original "can't pan" bug).
    val maxPanState = rememberUpdatedState(maxPan)
    val horizontalState = rememberUpdatedState(horizontal)

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black)
            .clipToBounds()
            .pointerInput(Unit) {
                // Consume the drag along the pan axis in the INITIAL pass so the
                // parent HorizontalPager (in PhotoViewerScreen) does not see it
                // and try to flip pages.
                //
                // CRITICAL: only consume a change when it carries an actual
                // positionChange. The DOWN and UP events have a zero
                // positionChange — consuming those swallows taps and makes the
                // "Full View" toggle pill un-clickable.
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent(PointerEventPass.Initial)
                        var d = 0f
                        event.changes.forEach { change ->
                            if (change.pressed) {
                                val delta = if (horizontalState.value)
                                    change.positionChange().x else change.positionChange().y
                                if (delta != 0f) {
                                    d += delta
                                    change.consume()
                                }
                            }
                        }
                        if (d != 0f) {
                            pan = (pan - d).coerceIn(0f, maxPanState.value)
                        }
                    }
                }
            }
            .onSizeChanged { containerSize = Size(it.width.toFloat(), it.height.toFloat()) }
    ) {
        // Render the image at its natural aspect with the short side matching
        // the viewport, then translate in screen pixels. requiredWidth/Height
        // let it extend beyond the parent's max bounds; clipToBounds on the
        // parent clips the visible region. FillBounds does not distort because
        // the box already preserves the image aspect.
        val renderedWidthDp = with(density) { renderedWidthPx.toDp() }
        val renderedHeightDp = with(density) { renderedHeightPx.toDp() }
        // Compose CENTERS a child whose required size exceeds the parent (the Box
        // is TopStart, but an over-wide requiredWidth child still lands centered —
        // same gotcha as the gallery crop tile). So the image's leading edge sits
        // at -maxPan/2, not 0. Web positions its pan div at top-0/left-0 and
        // translates by -pan; to match that "pan=0 = leading edge" we shift by
        // +maxPan/2. Without this you start mid-pano, can only pan one way, and
        // overshoot into the black letterbox at the far end.
        val lead = maxPan / 2f
        AsyncImage(
            // Decode wide panoramas at a CAPPED size, not ORIGINAL. A full-res
            // pano (e.g. 17000×2540) decodes to a >100 MB bitmap that crashes on
            // draw ("Canvas: trying to draw too large bitmap"). Capping the
            // longest side to MAX_PANO_DECODE_PX keeps the bitmap within the GPU
            // budget while still allowing full panning. allowHardware(false)
            // keeps the AVIF/HEIF software decode path.
            model = ImageRequest.Builder(context)
                .data(imageData)
                .size(MAX_PANO_DECODE_PX)
                .allowHardware(false)
                .crossfade(true)
                .build(),
            contentDescription = contentDescription,
            contentScale = ContentScale.FillBounds,
            modifier = Modifier
                .requiredWidth(renderedWidthDp)
                .requiredHeight(renderedHeightDp)
                .graphicsLayer {
                    translationX = if (horizontal) lead - pan else 0f
                    translationY = if (horizontal) 0f else lead - pan
                }
        )

        // Pan position indicator (horizontal bar showing progress along the axis)
        if (maxPan > 0f && renderedSpan > 0f) {
            Box(
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 100.dp)
                    .height(3.dp)
                    .width(120.dp)
                    .clip(RoundedCornerShape(2.dp))
                    .background(Color.White.copy(alpha = 0.25f))
            ) {
                val frac = (pan / renderedSpan).coerceIn(0f, 1f)
                val barWidthFrac = (viewportSpan / renderedSpan).coerceIn(0f, 1f)
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
@androidx.annotation.OptIn(UnstableApi::class)
@Composable
fun MotionPhotoOverlay(
    photo: PhotoEntity,
    viewModel: PhotoViewerViewModel,
) {
    // Triggered by subtype=="motion" (see caller), not motion_video_blob_id:
    // Android-captured motion photos embed the video in the JPEG and carry NO
    // separate blob, so the old `motionVideoBlobId != null` gate hid the LIVE
    // button entirely. The server resolves the video by photo id.
    val serverPhotoId = photo.serverPhotoId ?: return
    val context = LocalContext.current
    var playing by remember(photo.localId) { mutableStateOf(true) }
    var loading by remember(photo.localId) { mutableStateOf(true) }
    var localUri by remember(photo.localId) { mutableStateOf<Uri?>(null) }
    var error by remember(photo.localId) { mutableStateOf(false) }

    // Fetch the motion video from `/api/photos/{id}/motion-video`, which returns
    // a ready-to-play MP4 (the server serves the stored blob if present, else
    // extracts the trailer from the server-side-decrypted photo). This mirrors
    // the web's MotionVideoOverlay exactly — no client-side decryption, and it
    // works for embedded motion videos that have no separate blob.
    LaunchedEffect(photo.localId, serverPhotoId) {
        try {
            val tmp = withContext(Dispatchers.IO) {
                val out = java.io.File(context.cacheDir, "motion_${photo.localId}.mp4")
                viewModel.downloadMotionVideoToFile(serverPhotoId, out)
                out
            }
            localUri = Uri.fromFile(tmp)
            loading = false
        } catch (e: Throwable) {
            Log.w("MotionPhotoOverlay", "motion video fetch failed: ${e.message}")
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

// ─── Burst filmstrip overlay ────────────────────────────────────────────────

/**
 * Horizontal filmstrip for browsing the frames of a burst photo, shown at the
 * bottom of the viewer. Mirrors the web's [BurstStrip].
 *
 * The pager itself only swipes between burst COVERS (one page per burst), so
 * this strip is the only way to step through the individual shots of a large
 * burst (e.g. 46 frames). Tapping a frame swaps the displayed image in place
 * via [onSelectFrame] without leaving the current pager page.
 */
@Composable
fun BurstStripOverlay(
    frames: List<PhotoEntity>,
    currentPhotoId: String,
    visible: Boolean,
    onSelectFrame: (String) -> Unit,
) {
    if (!visible || frames.size <= 1) return

    val listState = rememberLazyListState()

    // Keep the active frame scrolled into view as the selection moves.
    LaunchedEffect(currentPhotoId, frames) {
        val idx = frames.indexOfFirst { it.localId == currentPhotoId }
        if (idx >= 0) {
            try { listState.animateScrollToItem(idx) } catch (_: Exception) {}
        }
    }

    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.BottomCenter
    ) {
        androidx.compose.material3.Surface(
            modifier = Modifier
                .padding(bottom = 130.dp)
                .widthIn(max = 340.dp)
                .clip(RoundedCornerShape(12.dp)),
            color = Color.Black.copy(alpha = 0.7f),
            shape = RoundedCornerShape(12.dp)
        ) {
            LazyRow(
                state = listState,
                modifier = Modifier.padding(8.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                itemsIndexed(frames, key = { _, f -> f.localId }) { idx, frame ->
                    val isActive = frame.localId == currentPhotoId
                    val model: Any? = when {
                        frame.thumbnailPath != null -> File(frame.thumbnailPath!!)
                        frame.localPath != null -> Uri.parse(frame.localPath)
                        else -> null
                    }
                    Box(
                        modifier = Modifier
                            .size(48.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .border(
                                width = 2.dp,
                                color = if (isActive) Color.White else Color.Transparent,
                                shape = RoundedCornerShape(8.dp)
                            )
                            .clickable { onSelectFrame(frame.localId) },
                        contentAlignment = Alignment.Center
                    ) {
                        if (model != null) {
                            AsyncImage(
                                model = model,
                                contentDescription = "Burst frame ${idx + 1}",
                                contentScale = ContentScale.Crop,
                                modifier = Modifier.fillMaxSize()
                            )
                        } else {
                            Box(
                                modifier = Modifier
                                    .fillMaxSize()
                                    .background(Color.Gray.copy(alpha = 0.4f)),
                                contentAlignment = Alignment.Center
                            ) {
                                Text("${idx + 1}", color = Color.White, fontSize = 10.sp)
                            }
                        }
                    }
                }
            }
        }
    }
}

// ─── helper: end of file ───────────────────────────────────────────────────
