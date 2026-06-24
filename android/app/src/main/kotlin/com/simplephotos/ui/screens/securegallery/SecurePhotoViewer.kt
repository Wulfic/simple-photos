package com.simplephotos.ui.screens.securegallery

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import com.simplephotos.ui.components.rememberThumbnailRequest
import com.simplephotos.data.remote.dto.SecureGalleryItem
import com.simplephotos.ui.screens.viewer.MAX_PANO_DECODE_PX
import com.simplephotos.ui.screens.viewer.PanoramaOverlay
import com.simplephotos.ui.screens.viewer.describeImageBytes
import android.net.Uri
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import kotlinx.coroutines.Dispatchers
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
    onRemove: ((SecureGalleryItem) -> Unit)? = null
) {
    val pagerState = rememberPagerState(
        initialPage = initialIndex.coerceIn(0, (items.size - 1).coerceAtLeast(0)),
        pageCount = { items.size }
    )
    var confirmRemove by remember { mutableStateOf(false) }
    // When a panorama / 360 page enters Live (pan) mode we must stop the pager
    // from stealing the horizontal drag (otherwise panning flips pages). Reset
    // whenever the page changes so a swipe away always re-enables paging.
    var panoLive by remember { mutableStateOf(false) }
    LaunchedEffect(pagerState.currentPage) { panoLive = false }

    if (confirmRemove) {
        val current = items.getOrNull(pagerState.currentPage)
        AlertDialog(
            onDismissRequest = { confirmRemove = false },
            title = { Text("Remove from secure album?") },
            text = { Text("The photo will return to your regular gallery.") },
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
                item = items[page],
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

        // Remove-from-album overlay (mirrors web's per-item removal)
        if (onRemove != null && items.isNotEmpty()) {
            IconButton(
                onClick = { confirmRemove = true },
                modifier = Modifier
                    .statusBarsPadding()
                    .padding(8.dp)
                    .align(Alignment.TopEnd)
            ) {
                Icon(
                    Icons.Default.Delete,
                    contentDescription = "Remove from album",
                    tint = Color.White
                )
            }
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
                prepare()
                playWhenReady = false
            }
        }
    }
    DisposableEffect(player) { onDispose { player?.release() } }

    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        when {
            loading -> CircularProgressIndicator(color = Color.White)
            failed || player == null -> Text("Unable to play this video", color = Color.White)
            else -> AndroidView(
                modifier = Modifier.fillMaxSize(),
                factory = { ctx ->
                    PlayerView(ctx).apply {
                        this.player = player
                        useController = true
                    }
                }
            )
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
        // LIVE toggle pill (mirrors the main viewer's MotionPhotoOverlay)
        Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
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
