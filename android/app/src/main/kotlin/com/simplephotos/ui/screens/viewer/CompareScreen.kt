/**
 * Split-screen Compare (#21) — view two photos/videos side by side.
 *
 * Entered from the gallery multi-select bar when exactly two items are
 * selected. Reuses [PhotoPageContent] (the single-media pane used by the main
 * viewer) twice, so each pane gets the exact same decrypt→Coil / spblob→ExoPlayer
 * rendering plus INDEPENDENT pinch-zoom for free. Each pane owns its own
 * ExoPlayer so two videos can play at once. Layout adapts to orientation:
 * side-by-side in landscape, stacked in portrait.
 */
package com.simplephotos.ui.screens.viewer

import android.content.Context
import android.content.res.Configuration
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.repository.EncryptedBlobStream

/**
 * Build a dedicated ExoPlayer for a Compare pane. Mirrors the main viewer's
 * player config (local-first buffering, decoder fallback, loop) but is scoped
 * to a single pane so two videos can play simultaneously. The
 * [MediaBlobDataSource] factory range-decrypts encrypted video blobs
 * (`spblob://`, issue #17) so playback never buffers the whole file.
 */
@androidx.annotation.OptIn(UnstableApi::class)
private fun buildComparePlayer(context: Context, stream: EncryptedBlobStream): ExoPlayer {
    val mediaSourceFactory = DefaultMediaSourceFactory(
        MediaBlobDataSource.Factory(context, stream)
    )
    val loadControl = DefaultLoadControl.Builder()
        .setBufferDurationsMs(2_500, 30_000, 1_000, 2_000)
        .setTargetBufferBytes(2_500_000)
        .setPrioritizeTimeOverSizeThresholds(false)
        .build()
    val renderersFactory = DefaultRenderersFactory(context)
        .setEnableDecoderFallback(true)
        .setExtensionRendererMode(DefaultRenderersFactory.EXTENSION_RENDERER_MODE_PREFER)
    return ExoPlayer.Builder(context)
        .setMediaSourceFactory(mediaSourceFactory)
        .setLoadControl(loadControl)
        .setRenderersFactory(renderersFactory)
        .build()
        .apply {
            playWhenReady = false
            // Loop videos in Compare (parity with the main viewer, #22).
            repeatMode = Player.REPEAT_MODE_ONE
            // Muted by default: two panes can both be videos, so autoplaying
            // audio from both at once is cacophony. Each pane's controls have a
            // mute toggle to un-mute that pane on demand.
            volume = 0f
            videoChangeFrameRateStrategy = C.VIDEO_CHANGE_FRAME_RATE_STRATEGY_OFF
        }
}

@androidx.annotation.OptIn(UnstableApi::class)
@Composable
fun CompareScreen(
    firstId: String,
    secondId: String,
    onBack: () -> Unit,
) {
    val viewModel: PhotoViewerViewModel = hiltViewModel()
    val context = LocalContext.current

    // Load the two photos by local id (this VM otherwise pages the gallery;
    // Compare ignores that and shows exactly these two).
    var first by remember(firstId) { mutableStateOf<PhotoEntity?>(null) }
    var second by remember(secondId) { mutableStateOf<PhotoEntity?>(null) }
    var loadError by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(firstId, secondId) {
        try {
            first = viewModel.loadPhotoByLocalId(firstId)
            second = viewModel.loadPhotoByLocalId(secondId)
            if (first == null || second == null) loadError = "Photo not available"
        } catch (e: Throwable) {
            loadError = e.message ?: "Failed to load photos"
        }
    }

    // One ExoPlayer per pane so two videos can play at once. Released on dispose.
    val playerA = remember { buildComparePlayer(context, viewModel.encryptedBlobStream) }
    val playerB = remember { buildComparePlayer(context, viewModel.encryptedBlobStream) }
    DisposableEffect(Unit) {
        onDispose {
            playerA.stop(); playerA.release()
            playerB.stop(); playerB.release()
        }
    }

    val isLandscape =
        LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE

    Column(modifier = Modifier.fillMaxSize().background(Color.Black)) {
        // Slim top bar
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .statusBarsPadding()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back", tint = Color.White)
            }
            Text("Compare", color = Color.White, fontWeight = FontWeight.Medium, fontSize = 16.sp)
        }

        val paneA = first
        val paneB = second
        when {
            loadError != null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(loadError ?: "Error", color = Color.White)
            }
            paneA == null || paneB == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(color = Color.White)
            }
            isLandscape -> Row(Modifier.fillMaxSize()) {
                ComparePane(paneA, viewModel, playerA, "1", Modifier.weight(1f).fillMaxHeight())
                Spacer(Modifier.width(1.dp).fillMaxHeight().background(Color.White.copy(alpha = 0.15f)))
                ComparePane(paneB, viewModel, playerB, "2", Modifier.weight(1f).fillMaxHeight())
            }
            else -> Column(Modifier.fillMaxSize()) {
                ComparePane(paneA, viewModel, playerA, "1", Modifier.weight(1f).fillMaxWidth())
                Spacer(Modifier.height(1.dp).fillMaxWidth().background(Color.White.copy(alpha = 0.15f)))
                ComparePane(paneB, viewModel, playerB, "2", Modifier.weight(1f).fillMaxWidth())
            }
        }
    }
}

/**
 * A single Compare pane. Wraps [PhotoPageContent] as an always-active page with
 * its own ExoPlayer, so its pinch-zoom and (if a video) playback are fully
 * independent of the other pane.
 */
@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun ComparePane(
    photo: PhotoEntity,
    viewModel: PhotoViewerViewModel,
    player: ExoPlayer,
    badge: String,
    modifier: Modifier = Modifier,
) {
    var activeVideoUri by remember { mutableStateOf<Uri?>(null) }
    var playerError by remember { mutableStateOf<String?>(null) }

    DisposableEffect(player) {
        val listener = object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                playerError = error.message ?: "Cannot play this video"
            }
        }
        player.addListener(listener)
        onDispose { player.removeListener(listener) }
    }

    Box(modifier.background(Color.Black)) {
        PhotoPageContent(
            photo = photo,
            serverBaseUrl = viewModel.serverBaseUrl,
            viewModel = viewModel,
            okHttpClient = viewModel.okHttpClient,
            isActivePage = true,
            sharedPlayer = player,
            activeVideoUri = activeVideoUri,
            onVideoUriReady = { uri, _ ->
                if (uri != activeVideoUri) {
                    playerError = null
                    activeVideoUri = uri
                    player.setMediaItem(MediaItem.fromUri(uri))
                    player.prepare()
                    player.playWhenReady = true
                }
            },
            playerError = playerError,
        )
        // Pane badge (1 / 2)
        Box(
            modifier = Modifier
                .align(Alignment.TopStart)
                .padding(8.dp)
                .size(24.dp)
                .clip(CircleShape)
                .background(Color.Black.copy(alpha = 0.6f)),
            contentAlignment = Alignment.Center,
        ) {
            Text(badge, color = Color.White, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        }
    }
}
