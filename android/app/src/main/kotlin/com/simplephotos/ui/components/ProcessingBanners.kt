/**
 * Conversion and encryption progress banners for the gallery screen.
 *
 * [ConversionBanner] polls `GET /api/admin/conversion-status` and shows a
 * progress bar while media conversion (HEIC→JPEG, etc.) is active.
 *
 * [EncryptionBanner] polls the encrypted-sync endpoint and shows progress
 * while photos are being encrypted.  It suppresses itself while conversion
 * is active so the user sees only one banner at a time.
 */
package com.simplephotos.ui.components

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.simplephotos.data.remote.ApiService
import kotlinx.coroutines.delay

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

    AnimatedVisibility(visible = active && !dismissed, enter = fadeIn(), exit = fadeOut()) {
        val progress = if (total > 0) done.toFloat() / total else 0f
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 4.dp),
            shape = MaterialTheme.shapes.small,
            tonalElevation = 2.dp,
            color = Color(0xFFFFF3E0) // light orange
        ) {
            Column(modifier = Modifier.padding(12.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                        color = Color(0xFFE65100)
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        "Converting media… $done/$total",
                        style = MaterialTheme.typography.bodySmall,
                        color = Color(0xFFE65100),
                        modifier = Modifier.weight(1f)
                    )
                    Text(
                        "✕",
                        modifier = Modifier.clickable { dismissed = true },
                        color = Color(0xFFE65100)
                    )
                }
                Spacer(Modifier.height(4.dp))
                LinearProgressIndicator(
                    progress = { progress },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(4.dp),
                    color = Color(0xFFE65100),
                    trackColor = Color(0xFFFFE0B2)
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

    AnimatedVisibility(visible = visible, enter = fadeIn(), exit = fadeOut()) {
        val progress = if (batchTotal > 0) batchDone.toFloat() / batchTotal else 0f
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 4.dp),
            shape = MaterialTheme.shapes.small,
            tonalElevation = 2.dp,
            color = Color(0xFFE3F2FD) // light blue
        ) {
            Column(modifier = Modifier.padding(12.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                        color = Color(0xFF1565C0)
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        "Encrypting photos… $batchDone/$batchTotal",
                        style = MaterialTheme.typography.bodySmall,
                        color = Color(0xFF1565C0),
                        modifier = Modifier.weight(1f)
                    )
                    Text(
                        "✕",
                        modifier = Modifier.clickable { dismissed = true },
                        color = Color(0xFF1565C0)
                    )
                }
                Spacer(Modifier.height(4.dp))
                LinearProgressIndicator(
                    progress = { progress },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(4.dp),
                    color = Color(0xFF1565C0),
                    trackColor = Color(0xFFBBDEFB)
                )
            }
        }
    }
}
