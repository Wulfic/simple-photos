/**
 * Floating progress banners matching the web app's styling.
 *
 * All banners render as white (dark: gray-800) cards with colored spinners
 * and progress bars, positioned by their caller at the bottom of the screen
 * with a high z-index so they float above all other content.
 *
 * [ConversionBanner] polls `GET /api/admin/conversion-status` and shows a
 * progress bar while media conversion (HEIC→JPEG, etc.) is active.
 *
 * [EncryptionBanner] polls the encrypted-sync endpoint and shows progress
 * while photos are being encrypted.  It suppresses itself while conversion
 * is active so the user sees only one banner at a time.
 *
 * [RenderingCopyBanner] shows "Rendering edited copy…" while a server-side
 * duplicate render is in progress.
 */
package com.simplephotos.ui.components

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.simplephotos.data.remote.ApiService
import kotlinx.coroutines.delay

// ── Web-aligned color tokens ─────────────────────────────────────────────────

private val BannerBgLight = Color.White
private val BannerBgDark = Color(0xFF1F2937)   // gray-800
private val BannerBorderLight = Color(0xFFE5E7EB) // gray-200
private val BannerBorderDark = Color(0xFF374151)   // gray-700
private val BannerTextLight = Color(0xFF374151)    // gray-700
private val BannerTextDark = Color(0xFFE5E7EB)     // gray-200
private val DismissLight = Color(0xFF9CA3AF)       // gray-400
private val DismissDark = Color(0xFF6B7280)        // gray-500
private val TrackLight = Color(0xFFE5E7EB)         // gray-200
private val TrackDark = Color(0xFF374151)           // gray-700

private val OrangeLight = Color(0xFFF97316)        // orange-500
private val OrangeDark = Color(0xFFFB923C)         // orange-400
private val BlueLight = Color(0xFF3B82F6)          // blue-500
private val BlueDark = Color(0xFF60A5FA)           // blue-400

private val BannerShape = RoundedCornerShape(12.dp)

// ── Conversion Banner ────────────────────────────────────────────────────────

/**
 * Polls `/api/admin/conversion-status` every 2 seconds and shows a progress
 * bar while media conversion is active.
 */
@Composable
fun ConversionBanner(api: ApiService) {
    var active by remember { mutableStateOf(false) }
    var total by remember { mutableIntStateOf(0) }
    var done by remember { mutableIntStateOf(0) }
    var dismissed by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        while (true) {
            try {
                val status = api.getConversionStatus()
                active = status.active && status.total > 0
                total = status.total
                done = status.done.coerceAtMost(status.total)
                // Reset dismissal when a new batch starts
                if (!active) dismissed = false
            } catch (_: Exception) {
                active = false
            }
            delay(2_000)
        }
    }

    val dark = isSystemInDarkTheme()
    val accent = if (dark) OrangeDark else OrangeLight

    AnimatedVisibility(visible = active && !dismissed, enter = fadeIn(), exit = fadeOut()) {
        val progress = if (total > 0) done.toFloat() / total else 0f
        BannerCard(dark) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(20.dp),
                    strokeWidth = 2.dp,
                    color = accent,
                    trackColor = if (dark) Color(0xFF6B7280) else Color(0xFFD1D5DB)
                )
                Spacer(Modifier.width(12.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        "Converting media… $done/$total",
                        style = MaterialTheme.typography.bodySmall.copy(
                            fontSize = 14.sp
                        ),
                        color = if (dark) BannerTextDark else BannerTextLight
                    )
                    Spacer(Modifier.height(6.dp))
                    LinearProgressIndicator(
                        progress = { progress },
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(6.dp),
                        color = accent,
                        trackColor = if (dark) TrackDark else TrackLight
                    )
                }
                Spacer(Modifier.width(12.dp))
                Text(
                    "✕",
                    modifier = Modifier.clickable { dismissed = true },
                    color = if (dark) DismissDark else DismissLight,
                    fontSize = 16.sp
                )
            }
        }
    }
}

// ── Encryption Banner ────────────────────────────────────────────────────────

/**
 * Tracks encryption progress by polling encrypted-sync and counting items
 * without an encrypted blob. Suppressed while conversion is active.
 */
@Composable
fun EncryptionBanner(api: ApiService) {
    var pending by remember { mutableIntStateOf(0) }
    var batchTotal by remember { mutableIntStateOf(0) }
    var conversionActive by remember { mutableStateOf(false) }
    var dismissed by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        while (true) {
            try {
                // Check if conversion is active — suppress encryption banner during conversion
                val convStatus = try { api.getConversionStatus() } catch (_: Exception) { null }
                conversionActive = convStatus?.active == true && (convStatus.total > 0)

                if (!conversionActive) {
                    val syncResp = api.encryptedSync(after = null, limit = 500)
                    val unencrypted = syncResp.photos.count { it.encryptedBlobId.isNullOrEmpty() }
                    if (unencrypted > 0 && pending == 0) {
                        // New batch starting
                        batchTotal = unencrypted
                    }
                    pending = unencrypted
                    if (pending == 0) {
                        batchTotal = 0
                        dismissed = false
                    }
                }
            } catch (_: Exception) {
                pending = 0
            }
            delay(2_000)
        }
    }

    val visible = pending > 0 && !conversionActive && !dismissed
    val batchDone = (batchTotal - pending).coerceAtLeast(0)
    val dark = isSystemInDarkTheme()
    val accent = if (dark) BlueDark else BlueLight

    AnimatedVisibility(visible = visible, enter = fadeIn(), exit = fadeOut()) {
        val progress = if (batchTotal > 0) batchDone.toFloat() / batchTotal else 0f
        BannerCard(dark) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(20.dp),
                    strokeWidth = 2.dp,
                    color = accent,
                    trackColor = if (dark) Color(0xFF6B7280) else Color(0xFFD1D5DB)
                )
                Spacer(Modifier.width(12.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        "Encrypting photos… $batchDone/$batchTotal",
                        style = MaterialTheme.typography.bodySmall.copy(
                            fontSize = 14.sp
                        ),
                        color = if (dark) BannerTextDark else BannerTextLight
                    )
                    Spacer(Modifier.height(6.dp))
                    LinearProgressIndicator(
                        progress = { progress },
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(6.dp),
                        color = accent,
                        trackColor = if (dark) TrackDark else TrackLight
                    )
                }
                Spacer(Modifier.width(12.dp))
                Text(
                    "✕",
                    modifier = Modifier.clickable { dismissed = true },
                    color = if (dark) DismissDark else DismissLight,
                    fontSize = 16.sp
                )
            }
        }
    }
}

// ── Rendering Copy Banner ────────────────────────────────────────────────────

/**
 * Non-dismissible banner shown while a server-side duplicate render is
 * in progress (e.g. video re-encode via ffmpeg).
 */
@Composable
fun RenderingCopyBanner(visible: Boolean) {
    val dark = isSystemInDarkTheme()
    val accent = if (dark) BlueDark else BlueLight

    AnimatedVisibility(visible = visible, enter = fadeIn(), exit = fadeOut()) {
        BannerCard(dark) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(20.dp),
                    strokeWidth = 2.dp,
                    color = accent,
                    trackColor = if (dark) Color(0xFF6B7280) else Color(0xFFD1D5DB)
                )
                Spacer(Modifier.width(12.dp))
                Text(
                    "Rendering edited copy…",
                    style = MaterialTheme.typography.bodySmall.copy(
                        fontSize = 14.sp
                    ),
                    color = if (dark) BannerTextDark else BannerTextLight
                )
            }
        }
    }
}

// ── Shared card wrapper ──────────────────────────────────────────────────────

@Composable
private fun BannerCard(dark: Boolean, content: @Composable () -> Unit) {
    Surface(
        modifier = Modifier
            .widthIn(max = 400.dp)
            .fillMaxWidth()
            .shadow(8.dp, BannerShape)
            .border(
                width = 1.dp,
                color = if (dark) BannerBorderDark else BannerBorderLight,
                shape = BannerShape
            ),
        shape = BannerShape,
        color = if (dark) BannerBgDark else BannerBgLight
    ) {
        Box(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            content()
        }
    }
}
