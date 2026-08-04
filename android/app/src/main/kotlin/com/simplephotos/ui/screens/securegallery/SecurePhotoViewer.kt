package com.simplephotos.ui.screens.securegallery

import android.app.Activity
import android.graphics.BitmapFactory
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import com.simplephotos.ui.components.rememberThumbnailRequest
import com.simplephotos.data.media.Rendition
import com.simplephotos.data.media.offerableRenditions
import com.simplephotos.data.media.shouldOfferPicker
import com.simplephotos.data.remote.dto.SecureGalleryItem
import com.simplephotos.data.remote.dto.toDomain
import com.simplephotos.ui.screens.viewer.MAX_PANO_DECODE_PX
import com.simplephotos.ui.screens.viewer.PanoramaOverlay
import com.simplephotos.ui.screens.viewer.VideoControlsOverlay
import com.simplephotos.ui.screens.viewer.describeImageBytes
import android.net.Uri
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import java.io.File

// ─────────────────────────────────────────────────────────────────────────────
// Secure Photo Viewer — full-screen pager for encrypted items only
// ─────────────────────────────────────────────────────────────────────────────

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun SecurePhotoViewer(
    items: List<SecureGalleryItem>,
    initialIndex: Int,
    viewModel: SecureGalleryViewModel,
    onBack: () -> Unit,
    onRemove: ((SecureGalleryItem) -> Unit)? = null,
    // The FULL (un-collapsed) item list, so the burst filmstrip can resolve
    // every frame of a burst (the pager only swipes between covers).
    allItems: List<SecureGalleryItem> = items,
) {
    val pagerState = rememberPagerState(
        initialPage = initialIndex.coerceIn(0, (items.size - 1).coerceAtLeast(0)),
        pageCount = { items.size }
    )
    var confirmRemove by remember { mutableStateOf(false) }
    var overflowOpen by remember { mutableStateOf(false) }
    // Info panel + Download parity with the regular viewer (#31).
    var showInfo by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    // When a panorama / 360 page enters Live (pan) mode we must stop the pager
    // from stealing the horizontal drag (otherwise panning flips pages). Reset
    // whenever the page changes so a swipe away always re-enables paging.
    var panoLive by remember { mutableStateOf(false) }
    LaunchedEffect(pagerState.currentPage) { panoLive = false }

    // Immersive full-screen parity with the regular viewer
    // (PhotoViewerScreen): hide the system bars so the video controls (and the
    // back/remove buttons) aren't overlapped by the phone's on-screen nav bar.
    // Bars return transiently on swipe; controls carry navigationBarsPadding so
    // they stay reachable then (todo1 issue 1).
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

    // Per-burst selected frame (cover-itemId → frame-itemId). The pager swipes
    // between burst covers; selecting a frame in the filmstrip swaps the image
    // shown on that page WITHOUT making each frame its own page.
    var burstSelections by remember { mutableStateOf<Map<String, String>>(emptyMap()) }

    fun framesFor(cover: SecureGalleryItem): List<SecureGalleryItem> {
        val bid = cover.burstId
        return if (bid.isNullOrEmpty()) emptyList() else allItems.filter { it.burstId == bid }
    }

    fun effectiveItem(cover: SecureGalleryItem): SecureGalleryItem {
        val frames = framesFor(cover)
        if (frames.isEmpty()) return cover
        val selId = burstSelections[cover.id]
        return frames.firstOrNull { it.id == selId } ?: cover
    }

    // Save a decrypted secure item to the device Downloads folder (#31). Mirrors
    // the regular viewer's MediaStore write; the decrypted plaintext leaves the
    // app only on this explicit, user-initiated export.
    fun downloadCurrent(item: SecureGalleryItem) {
        scope.launch {
            try {
                val bytes = viewModel.downloadAndDecrypt(item.blobId)
                val ext = when (item.mediaType) {
                    "video" -> "mp4"; "gif" -> "gif"; "audio" -> "m4a"; else -> "jpg"
                }
                val mime = when (item.mediaType) {
                    "video" -> "video/mp4"; "gif" -> "image/gif"; "audio" -> "audio/mp4"
                    else -> "image/jpeg"
                }
                val name = "secure_${item.id.take(8)}.$ext"
                val values = android.content.ContentValues().apply {
                    put(android.provider.MediaStore.MediaColumns.DISPLAY_NAME, name)
                    put(android.provider.MediaStore.MediaColumns.MIME_TYPE, mime)
                    if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
                        put(
                            android.provider.MediaStore.MediaColumns.RELATIVE_PATH,
                            android.os.Environment.DIRECTORY_DOWNLOADS,
                        )
                    }
                }
                val collection =
                    if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q)
                        android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI
                    else android.provider.MediaStore.Images.Media.EXTERNAL_CONTENT_URI
                val uri = context.contentResolver.insert(collection, values)
                if (uri == null) {
                    android.widget.Toast.makeText(context, "Download failed", android.widget.Toast.LENGTH_SHORT).show()
                    return@launch
                }
                context.contentResolver.openOutputStream(uri)?.use { it.write(bytes) }
                android.widget.Toast.makeText(context, "Saved to Downloads", android.widget.Toast.LENGTH_SHORT).show()
            } catch (e: Exception) {
                android.util.Log.e("SecurePhotoViewer", "download failed for ${item.blobId}", e)
                android.widget.Toast.makeText(context, "Download failed", android.widget.Toast.LENGTH_SHORT).show()
            }
        }
    }

    if (showInfo) {
        val current = items.getOrNull(pagerState.currentPage)?.let { effectiveItem(it) }
        AlertDialog(
            onDismissRequest = { showInfo = false },
            confirmButton = { TextButton(onClick = { showInfo = false }) { Text("Close") } },
            title = { Text("Info") },
            text = {
                Column {
                    val rows = buildList {
                        current?.mediaType?.takeIf { it.isNotEmpty() }?.let { add("Type" to it) }
                        current?.photoSubtype?.takeIf { it.isNotEmpty() }?.let { add("Subtype" to it) }
                        val w = current?.width; val h = current?.height
                        if (w != null && h != null && w > 0 && h > 0) add("Dimensions" to "$w × $h")
                        current?.durationSecs?.let { d ->
                            val m = (d / 60).toInt(); val s = (d % 60).toInt()
                            add("Duration" to "$m:${s.toString().padStart(2, '0')}")
                        }
                        current?.addedAt?.takeIf { it.isNotEmpty() }?.let { add("Added" to it) }
                        if (current?.cropMetadata != null) add("Edited" to "Yes (cropped)")
                    }
                    rows.forEach { (label, value) ->
                        Text("$label: $value", fontSize = 13.sp)
                    }
                }
            }
        )
    }

    if (confirmRemove) {
        val current = items.getOrNull(pagerState.currentPage)
        val isBurst = current?.let { framesFor(it).size > 1 } == true
        AlertDialog(
            onDismissRequest = { confirmRemove = false },
            title = { Text("Remove from secure album?") },
            text = {
                Text(
                    if (isBurst)
                        "This burst (all of its frames) will return to your regular gallery."
                    else
                        "The photo will return to your regular gallery."
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmRemove = false
                    current?.let { onRemove?.invoke(it) }
                }) { Text("Remove") }
            },
            dismissButton = {
                TextButton(onClick = { confirmRemove = false }) { Text("Cancel") }
            }
        )
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black)
    ) {
        HorizontalPager(
            state = pagerState,
            userScrollEnabled = !panoLive,
            modifier = Modifier.fillMaxSize()
        ) { page ->
            SecureMediaPage(
                item = effectiveItem(items[page]),
                viewModel = viewModel,
                onPanoLiveModeChange = { live ->
                    if (pagerState.currentPage == page) panoLive = live
                }
            )
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

        // Overflow (⋮) menu — Info + Download (read-only parity with the
        // regular/web viewers, #31) plus Remove when the album is editable.
        if (items.isNotEmpty()) {
            Box(
                modifier = Modifier
                    .statusBarsPadding()
                    .padding(8.dp)
                    .align(Alignment.TopEnd)
            ) {
                IconButton(onClick = { overflowOpen = true }) {
                    Icon(
                        Icons.Default.MoreVert,
                        contentDescription = "More options",
                        tint = Color.White
                    )
                }
                MaterialTheme(
                    shapes = MaterialTheme.shapes.copy(extraSmall = RoundedCornerShape(16.dp))
                ) {
                    DropdownMenu(
                        expanded = overflowOpen,
                        onDismissRequest = { overflowOpen = false }
                    ) {
                        DropdownMenuItem(
                            text = { Text("Info") },
                            onClick = { overflowOpen = false; showInfo = true },
                            leadingIcon = {
                                Icon(Icons.Default.Info, contentDescription = null, modifier = Modifier.size(18.dp))
                            }
                        )
                        DropdownMenuItem(
                            text = { Text("Download") },
                            onClick = {
                                overflowOpen = false
                                items.getOrNull(pagerState.currentPage)?.let { downloadCurrent(effectiveItem(it)) }
                            },
                            leadingIcon = {
                                Icon(Icons.Default.Download, contentDescription = null, modifier = Modifier.size(18.dp))
                            }
                        )
                        if (onRemove != null) {
                            DropdownMenuItem(
                                text = { Text("Remove from secure album", color = MaterialTheme.colorScheme.error) },
                                onClick = {
                                    overflowOpen = false
                                    confirmRemove = true
                                },
                                leadingIcon = {
                                    Icon(
                                        Icons.Default.Delete,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.error,
                                        modifier = Modifier.size(18.dp)
                                    )
                                }
                            )
                        }
                    }
                }
            }
        }

        // Burst filmstrip — step through the frames of the current burst.
        val curCover = items.getOrNull(pagerState.currentPage)
        if (curCover != null && !panoLive) {
            val frames = framesFor(curCover)
            if (frames.size > 1) {
                SecureBurstStrip(
                    frames = frames,
                    currentItemId = effectiveItem(curCover).id,
                    viewModel = viewModel,
                    onSelect = { fid -> burstSelections = burstSelections + (curCover.id to fid) }
                )
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Secure burst filmstrip — horizontal frame picker for one burst
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Filmstrip shown at the bottom of the secure viewer while a burst is open.
 * The pager only swipes between burst COVERS, so this strip is the only way to
 * step through the individual frames of a secure burst. Tapping a frame swaps
 * the displayed image in place (via [onSelect]). Mirrors the regular viewer's
 * BurstStripOverlay and the web's BurstStrip.
 */
@Composable
private fun SecureBurstStrip(
    frames: List<SecureGalleryItem>,
    currentItemId: String,
    viewModel: SecureGalleryViewModel,
    onSelect: (String) -> Unit,
) {
    val listState = rememberLazyListState()
    LaunchedEffect(currentItemId, frames) {
        val idx = frames.indexOfFirst { it.id == currentItemId }
        if (idx >= 0) try { listState.animateScrollToItem(idx) } catch (_: Exception) {}
    }
    Box(
        modifier = Modifier.fillMaxSize().navigationBarsPadding(),
        contentAlignment = Alignment.BottomCenter
    ) {
        Surface(
            modifier = Modifier
                .padding(bottom = 24.dp)
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
                itemsIndexed(frames, key = { _, f -> f.id }) { idx, frame ->
                    SecureBurstThumb(
                        item = frame,
                        index = idx,
                        isActive = frame.id == currentItemId,
                        viewModel = viewModel,
                        onClick = { onSelect(frame.id) }
                    )
                }
            }
        }
    }
}

/** One decrypted 48dp thumbnail in the secure burst filmstrip. */
@Composable
private fun SecureBurstThumb(
    item: SecureGalleryItem,
    index: Int,
    isActive: Boolean,
    viewModel: SecureGalleryViewModel,
    onClick: () -> Unit,
) {
    var bitmap by remember(item.blobId) { mutableStateOf<android.graphics.Bitmap?>(null) }
    LaunchedEffect(item.blobId) {
        try {
            val data = viewModel.downloadThumb(item.blobId, item.encryptedThumbBlobId)
            bitmap = BitmapFactory.decodeByteArray(data, 0, data.size)
        } catch (_: Exception) {
            bitmap = null
        }
    }
    Box(
        modifier = Modifier
            .size(48.dp)
            .clip(RoundedCornerShape(8.dp))
            .border(2.dp, if (isActive) Color.White else Color.Transparent, RoundedCornerShape(8.dp))
            .background(Color.Gray.copy(alpha = 0.4f))
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center
    ) {
        val bmp = bitmap
        if (bmp != null) {
            Image(
                bitmap = bmp.asImageBitmap(),
                contentDescription = "Burst frame ${index + 1}",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
        } else {
            Text("${index + 1}", color = Color.White, fontSize = 10.sp)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Secure media page — type-aware renderer for one pager page
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Renders one secure item full-screen, branching on its type so the secure
 * viewer matches the main gallery:
 *   - video    → decrypt to a temp file, play with ExoPlayer + controls
 *   - pano/360 → still image + interactive [PanoramaOverlay] (reused from viewer)
 *   - motion   → still image + LIVE overlay (embedded MP4 extracted client-side)
 *   - photo/gif→ Coil image (Coil sniffs GIF / AVIF / etc.)
 *
 * Image types are decrypted to a ByteArray and handed to Coil, which downsamples
 * safely (panoramas capped to [MAX_PANO_DECODE_PX] to dodge the "too large
 * bitmap" crash). Videos / motion trailers go to disk and are wiped on dispose
 * so the decrypted plaintext doesn't linger in the cache.
 */
@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun SecureMediaPage(
    item: SecureGalleryItem,
    viewModel: SecureGalleryViewModel,
    onPanoLiveModeChange: (Boolean) -> Unit,
) {
    val sub = item.photoSubtype
    val isVideo = item.mediaType == "video"
    val isPano = sub == "panorama" || sub == "equirectangular"
    val isMotion = sub == "motion" && !isVideo

    if (isVideo) {
        SecureVideoPage(item, viewModel)
        return
    }

    val context = LocalContext.current
    var decrypted by remember(item.blobId) { mutableStateOf<ByteArray?>(null) }
    var loading by remember(item.blobId) { mutableStateOf(true) }
    var failed by remember(item.blobId) { mutableStateOf(false) }
    // Coil decode error (distinct from a decrypt failure). Previously a decode
    // failure on the base image was swallowed → pure black. Surface it so a
    // black 360/pano can be diagnosed instead of looking like a blank page.
    var imageError by remember(item.blobId) { mutableStateOf<String?>(null) }

    LaunchedEffect(item.blobId) {
        loading = true; failed = false
        try {
            val data = viewModel.downloadAndDecrypt(item.blobId)
            android.util.Log.d(
                "SecureMediaPage",
                "decrypted blobId=${item.blobId} sub=$sub → ${describeImageBytes(data)}"
            )
            // AVIF/HEIF are handled by the app's AvifCoilDecoder (libavif), so the
            // raw decrypted bytes can go straight to Coil — no temp file / no
            // plaintext on disk.
            decrypted = data
        } catch (e: Exception) {
            android.util.Log.e("SecureMediaPage", "decrypt failed blobId=${item.blobId}", e)
            failed = true
        } finally {
            loading = false
        }
    }

    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        when {
            loading -> CircularProgressIndicator(color = Color.White)
            failed || decrypted == null -> Text("Failed to decrypt", color = Color.White)
            else -> {
                val data = decrypted!!
                AsyncImage(
                    // Capped decode (NOT ORIGINAL) for wide panos/360 — see MAX_PANO_DECODE_PX.
                    model = rememberThumbnailRequest(
                        data = data,
                        size = if (isPano) MAX_PANO_DECODE_PX else null,
                        allowHardware = !isPano,
                    ),
                    contentDescription = "Secure photo",
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Fit,
                    onState = { state ->
                        if (state is AsyncImagePainter.State.Error) {
                            val t = state.result.throwable
                            android.util.Log.w(
                                "SecureMediaPage",
                                "Coil decode failed blobId=${item.blobId} sub=$sub: ${t.message}",
                                t
                            )
                            imageError = "Unable to display this image"
                        } else if (state is AsyncImagePainter.State.Success) {
                            imageError = null
                        }
                    }
                )

                // Visible fallback for a base-image decode failure (was: black).
                if (imageError != null) {
                    Text(imageError!!, color = Color.White)
                }

                if (isPano) {
                    PanoramaOverlay(
                        imageData = data,
                        intrinsicWidth = (item.width ?: 0).toFloat(),
                        intrinsicHeight = (item.height ?: 0).toFloat(),
                        is360 = sub == "equirectangular",
                        contentDescription = "Secure panorama",
                        onLiveModeChange = { live, _ -> onPanoLiveModeChange(live) },
                    )
                } else if (isMotion) {
                    SecureMotionOverlay(jpegBytes = decrypted!!, blobKey = item.blobId)
                }
            }
        }
    }
}

/**
 * Plays a decrypted secure video. The blob is streamed-decrypted to a temp file
 * (ExoPlayer needs a file/URI, not a ByteArray) and deleted on dispose.
 *
 * ## The quality picker (#49 remainder)
 *
 * Unlike the main viewer, this page cannot stream a rung through
 * `MediaBlobDataSource` — the whole secure path is decrypt-to-a-temp-file — so a
 * quality switch is a **second download**, not a re-point. Three consequences,
 * each of which is a bug if skipped:
 *
 * 1. The playhead and play/pause state are captured before the swap and restored
 *    after, exactly as `VideoPlayer` does. A quality change that restarts the
 *    video reads as the player crashing.
 * 2. The *previous* file is deleted only once the new one exists, so a failed
 *    switch leaves the current quality playing rather than a blank page.
 * 3. Every decrypted file is wiped on dispose, including one produced by a
 *    switch. A rendition is as much plaintext as the original; leaving a 1080p
 *    copy in the cache dir would defeat the album as thoroughly as leaving the
 *    4K one.
 *
 * Selecting "Original" (`isSource`) means *this item's own payload*, not the
 * source rung's `blobId` — that id names the hidden original's blob, which is a
 * different object from the secure clone this page is showing.
 */
@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun SecureVideoPage(
    item: SecureGalleryItem,
    viewModel: SecureGalleryViewModel,
) {
    val context = LocalContext.current
    var videoFile by remember(item.blobId) { mutableStateOf<File?>(null) }
    var loading by remember(item.blobId) { mutableStateOf(true) }
    var failed by remember(item.blobId) { mutableStateOf(false) }

    // Which rung is on screen; null = this item's own payload ("Original").
    var selectedRendition by remember(item.blobId) { mutableStateOf<Rendition?>(null) }
    var switching by remember(item.blobId) { mutableStateOf(false) }
    // Playback state carried across a source swap.
    var pendingResume by remember(item.blobId) { mutableStateOf<Pair<Long, Boolean>?>(null) }

    val ladder = remember(item.blobId, item.renditions) { item.renditions.toDomain() }
    val offerable = remember(ladder) { offerableRenditions(ladder) }
    val hasPicker = remember(ladder) { shouldOfferPicker(ladder) }

    LaunchedEffect(item.blobId) {
        loading = true; failed = false
        try {
            videoFile = viewModel.downloadAndDecryptToFile(item.blobId, "mp4")
        } catch (e: Exception) {
            android.util.Log.e("SecureVideoPage", "decrypt video failed blobId=${item.blobId}", e)
            failed = true
        } finally {
            loading = false
        }
    }

    // Wipe the decrypted plaintext when leaving the page (confidentiality).
    DisposableEffect(videoFile) {
        val f = videoFile
        onDispose { f?.delete() }
    }

    val player = remember(videoFile) {
        videoFile?.let { f ->
            ExoPlayer.Builder(context).build().apply {
                setMediaItem(MediaItem.fromUri(Uri.fromFile(f)))
                // Loop instead of stopping after one play (#22), matching the
                // main gallery video viewer.
                repeatMode = Player.REPEAT_MODE_ALL
                prepare()
                playWhenReady = false
            }
        }
    }
    DisposableEffect(player) { onDispose { player?.release() } }

    // Restore the playhead once the swapped-in file is ready. Keyed on the file
    // rather than the player so it also covers the initial load being replaced.
    LaunchedEffect(player, videoFile) {
        val p = player ?: return@LaunchedEffect
        val resume = pendingResume ?: return@LaunchedEffect
        pendingResume = null
        val (position, wasPlaying) = resume
        // A rendition has the same duration as its source, so the playhead
        // transfers directly. Clamped anyway: #46's salvage re-encode of a
        // corrupt source is legitimately shorter than the original.
        val duration = p.duration
        p.seekTo(if (duration > 0) minOf(position, duration - 100L).coerceAtLeast(0L) else position)
        p.playWhenReady = wasPlaying
    }

    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        when {
            loading -> CircularProgressIndicator(color = Color.White)
            failed || player == null -> Text("Unable to play this video", color = Color.White)
            else -> {
                val activePlayer = player
                // Custom controls parity with the regular viewer
                // (VideoPlayer.kt): a raw TextureView + the shared
                // VideoControlsOverlay instead of media3's stock controller,
                // whose settings-gear button rendered under the on-screen nav
                // bar and was unclickable (todo1 issue 1). TextureView also
                // avoids SurfaceView's transform issues.
                var showControls by remember { mutableStateOf(true) }
                val playerIsPlaying = remember { mutableStateOf(activePlayer.isPlaying) }
                DisposableEffect(activePlayer) {
                    val listener = object : Player.Listener {
                        override fun onIsPlayingChanged(playing: Boolean) {
                            playerIsPlaying.value = playing
                        }
                    }
                    activePlayer.addListener(listener)
                    onDispose { activePlayer.removeListener(listener) }
                }
                // Auto-hide the controls 3s after they appear while playing.
                LaunchedEffect(showControls, playerIsPlaying.value) {
                    if (showControls && playerIsPlaying.value) {
                        delay(3000)
                        showControls = false
                    }
                }

                AndroidView(
                    modifier = Modifier.fillMaxSize(),
                    factory = { ctx -> android.view.TextureView(ctx) },
                    update = { v -> activePlayer.setVideoTextureView(v) }
                )

                // Tap catcher toggles the controls (sits above the video,
                // below the controls overlay so its widgets stay tappable).
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .clickable(
                            interactionSource = remember { MutableInteractionSource() },
                            indication = null
                        ) { showControls = !showControls }
                )

                val scope = rememberCoroutineScope()
                VideoControlsOverlay(
                    player = activePlayer,
                    visible = showControls,
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .navigationBarsPadding(),
                    renditions = if (hasPicker) offerable else emptyList(),
                    selectedRendition = selectedRendition,
                    onSelectRendition = { target ->
                        val alreadyOnSource = selectedRendition == null && target.isSource
                        val alreadyOnTarget = target.shortEdge == selectedRendition?.shortEdge
                        // Re-picking what is already playing must be a no-op, or
                        // the user pays a full re-download to arrive where they
                        // already were — and, worse, loses their place doing it.
                        if (switching || alreadyOnSource || alreadyOnTarget) return@VideoControlsOverlay

                        val resume = activePlayer.currentPosition to activePlayer.isPlaying
                        val previous = videoFile
                        // "Original" is this item's own payload, NOT the source
                        // rung's blobId (which names the hidden original's blob).
                        val targetBlob = if (target.isSource) item.blobId else target.blobId
                        if (targetBlob == null) {
                            // A null blobId means an unencrypted install, where
                            // the bytes live behind the file route. The secure
                            // path has no plaintext branch, and offerable rungs
                            // are filtered on exactly this — so reaching here is
                            // a bug in the filter, not a user-visible state.
                            android.util.Log.w(
                                "SecureVideoPage",
                                "rung ${target.shortEdge} has no blob to fetch; ignoring"
                            )
                            return@VideoControlsOverlay
                        }
                        switching = true
                        scope.launch {
                            try {
                                val next = viewModel.downloadAndDecryptToFile(targetBlob, "mp4")
                                pendingResume = resume
                                selectedRendition = target.takeIf { !it.isSource }
                                videoFile = next
                                // Only now: a failed switch must leave the
                                // current quality playing, not a deleted file.
                                if (previous != next) previous?.delete()
                            } catch (e: Exception) {
                                // Every failure path logs — a silent revert looks
                                // exactly like the picker being ignored, which is
                                // unreportable.
                                android.util.Log.e(
                                    "SecureVideoPage",
                                    "quality switch to ${target.shortEdge}p failed " +
                                        "(blobId=$targetBlob)",
                                    e
                                )
                            } finally {
                                switching = false
                            }
                        }
                    }
                )
            }
        }
    }
}

/**
 * Plays the motion-photo trailer embedded inside a decrypted JPEG, muted and
 * looping, on top of the still. The MP4 is extracted client-side (the secure
 * clone has no separate motion-video blob) and wiped on dispose. Renders
 * nothing extra if no embedded video is found — the still already shows.
 */
@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun SecureMotionOverlay(
    jpegBytes: ByteArray,
    blobKey: String,
) {
    val context = LocalContext.current
    var videoFile by remember(blobKey) { mutableStateOf<File?>(null) }
    var available by remember(blobKey) { mutableStateOf(true) }
    var playing by remember(blobKey) { mutableStateOf(true) }

    LaunchedEffect(blobKey) {
        val file = withContext(Dispatchers.IO) {
            val mp4 = extractEmbeddedMp4(jpegBytes) ?: return@withContext null
            File.createTempFile("secure_motion_", ".mp4", context.cacheDir).apply { writeBytes(mp4) }
        }
        if (file == null) available = false else videoFile = file
    }
    DisposableEffect(videoFile) {
        val f = videoFile
        onDispose { f?.delete() }
    }

    if (!available) return  // no embedded video — the still already shows

    val player = remember(videoFile) {
        videoFile?.let { f ->
            ExoPlayer.Builder(context).build().apply {
                setMediaItem(MediaItem.fromUri(Uri.fromFile(f)))
                repeatMode = Player.REPEAT_MODE_ALL
                volume = 0f
                prepare()
                playWhenReady = true
            }
        }
    }
    DisposableEffect(player) { onDispose { player?.release() } }
    LaunchedEffect(playing, player) { player?.playWhenReady = playing }

    Box(modifier = Modifier.fillMaxSize()) {
        if (player != null && playing) {
            AndroidView(
                modifier = Modifier.fillMaxSize(),
                factory = { ctx -> PlayerView(ctx).apply { useController = false; this.player = player } }
            )
        }
        // LIVE toggle pill (mirrors the main viewer's MotionPhotoOverlay).
        // navigationBarsPadding keeps it clear of transient bars now that the
        // secure viewer runs immersive (todo1 issue 1).
        Box(
            modifier = Modifier.fillMaxSize().navigationBarsPadding(),
            contentAlignment = Alignment.BottomCenter
        ) {
            Surface(
                modifier = Modifier
                    .padding(bottom = 80.dp)
                    .clip(androidx.compose.foundation.shape.CircleShape)
                    .clickable { playing = !playing },
                color = if (playing) Color.White else Color.Black.copy(alpha = 0.6f),
                shape = androidx.compose.foundation.shape.CircleShape
            ) {
                Text(
                    text = if (playing) "LIVE ●" else "LIVE ○",
                    color = if (playing) Color.Black else Color.White,
                    fontWeight = FontWeight.Bold,
                    fontSize = 12.sp,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp)
                )
            }
        }
    }
}

/**
 * Find an embedded MP4 trailer in a motion-photo JPEG by scanning for the
 * `ftyp` box signature (the ISO base-media marker). The MP4 begins 4 bytes
 * before `ftyp` (the box-size prefix). Mirrors the server's ftyp confirmation
 * in `extract_motion_video`. Returns null if no plausible trailer is found.
 */
private fun extractEmbeddedMp4(data: ByteArray): ByteArray? {
    var i = 4
    val end = data.size - 4
    while (i <= end) {
        if (data[i] == 'f'.code.toByte() && data[i + 1] == 't'.code.toByte() &&
            data[i + 2] == 'y'.code.toByte() && data[i + 3] == 'p'.code.toByte()
        ) {
            val start = i - 4
            // Require a real trailer to skip a stray 'ftyp' inside the JPEG data.
            if (start > 0 && data.size - start > 4096) {
                return data.copyOfRange(start, data.size)
            }
        }
        i++
    }
    return null
}
