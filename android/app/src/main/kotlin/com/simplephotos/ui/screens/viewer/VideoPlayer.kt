/**
 * Video player composables — ExoPlayer-backed video/audio playback with
 * custom controls overlay, extracted from PhotoViewerComponents.
 */
package com.simplephotos.ui.screens.viewer

import android.net.Uri
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import kotlinx.coroutines.delay

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

        // Track the actual video frame dimensions and any container rotation
        // that the TextureView doesn't auto-apply (unlike PlayerView/SurfaceView).
        var videoFrameW by remember { mutableIntStateOf(0) }
        var videoFrameH by remember { mutableIntStateOf(0) }
        var unappliedRotation by remember { mutableIntStateOf(0) }
        DisposableEffect(sharedPlayer) {
            val listener = object : androidx.media3.common.Player.Listener {
                override fun onVideoSizeChanged(videoSize: androidx.media3.common.VideoSize) {
                    videoFrameW = videoSize.width
                    videoFrameH = videoSize.height
                    @Suppress("DEPRECATION")
                    unappliedRotation = videoSize.unappliedRotationDegrees
                }
            }
            sharedPlayer.addListener(listener)
            // Check current state in case already ready
            val sz = sharedPlayer.videoSize
            if (sz.width > 0 && sz.height > 0) {
                videoFrameW = sz.width
                videoFrameH = sz.height
                @Suppress("DEPRECATION")
                unappliedRotation = sz.unappliedRotationDegrees
            }
            onDispose { sharedPlayer.removeListener(listener) }
        }

        // Combine user-requested rotation with any container rotation the
        // decoder didn't handle (unappliedRotationDegrees from the container).
        val totalRotation = (activeRotation + unappliedRotation) % 360

        // For 90/270 rotation, scale down so the rotated video fits the container.
        BoxWithConstraints(modifier = Modifier.fillMaxSize()) {
            val containerW = constraints.maxWidth.toFloat()
            val containerH = constraints.maxHeight.toFloat()
            val rot = totalRotation % 360
            val isSwapped = rot == 90 || rot == 270 || rot == -90 || rot == -270

            // Use the ORIGINAL (pre-rotation) frame aspect for the TextureView
            // because ExoPlayer delivers unrotated frames — the graphicsLayer
            // rotation on the parent Box handles the visual transform.
            val textureAspect = if (videoFrameW > 0 && videoFrameH > 0) {
                videoFrameW.toFloat() / videoFrameH.toFloat()
            } else if (photoWidth > 0 && photoHeight > 0) {
                photoWidth.toFloat() / photoHeight.toFloat()
            } else 0f

            val videoRotationModifier = if (totalRotation != 0) {
                if (isSwapped && videoFrameW > 0 && videoFrameH > 0 && containerW > 0 && containerH > 0) {
                    // Known media dimensions — precise scaling so the rotated video fits
                    val origAspect = videoFrameW.toFloat() / videoFrameH.toFloat()
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
                        rotationZ = totalRotation.toFloat()
                    }
                } else if (isSwapped && containerW > 0 && containerH > 0) {
                    // Unknown media dimensions — use container-based scale to prevent overflow
                    val rotScale = minOf(containerW / containerH, containerH / containerW)
                    Modifier.graphicsLayer {
                        scaleX = rotScale; scaleY = rotScale
                        rotationZ = totalRotation.toFloat()
                    }
                } else {
                    Modifier.graphicsLayer { rotationZ = totalRotation.toFloat() }
                }
            } else Modifier

            // Constrain the TextureView to the video's original aspect ratio
            // so it letterboxes correctly instead of stretching to fill.
            val textureModifier = if (textureAspect > 0f) {
                Modifier.aspectRatio(textureAspect, matchHeightConstraintsFirst = textureAspect < 1f)
            } else {
                Modifier.fillMaxSize()
            }

            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .then(videoRotationModifier),
                contentAlignment = Alignment.Center
            ) {
                // Use a raw TextureView instead of PlayerView's default SurfaceView.
                // SurfaceView creates a separate window surface that doesn't support
                // Compose graphicsLayer transforms (rotation, scale) — it crashes or
                // renders incorrectly on many devices. TextureView renders into a
                // regular texture that participates in normal view compositing.
                AndroidView(
                factory = { ctx ->
                    android.view.TextureView(ctx)
                },
                update = { view ->
                    sharedPlayer.setVideoTextureView(view)
                },
                modifier = textureModifier
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
                    modifier = textureModifier
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
