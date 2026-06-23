package com.simplephotos.ui.screens.securegallery

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.simplephotos.R
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.remote.dto.SecureGalleryItem

// ─────────────────────────────────────────────────────────────────────────────
// Secure Item Tile — downloads and shows decrypted thumbnail
// ─────────────────────────────────────────────────────────────────────────────

@Composable
internal fun SecureItemTile(
    item: SecureGalleryItem,
    onClick: () -> Unit,
    viewModel: SecureGalleryViewModel,
    burstCount: Int = 0
) {
    var bitmap by remember(item.blobId) { mutableStateOf<android.graphics.Bitmap?>(null) }
    var gifBytes by remember(item.blobId) { mutableStateOf<ByteArray?>(null) }
    var loading by remember(item.blobId) { mutableStateOf(true) }

    LaunchedEffect(item.blobId) {
        loading = true
        try {
            val data = viewModel.downloadThumb(item.blobId, item.encryptedThumbBlobId)
            // Detect GIF by magic bytes (GIF87a / GIF89a)
            val isGif = data.size > 3 &&
                data[0] == 0x47.toByte() && data[1] == 0x49.toByte() && data[2] == 0x46.toByte()
            if (isGif) {
                gifBytes = data
            } else {
                bitmap = BitmapFactory.decodeByteArray(data, 0, data.size)
            }
        } catch (e: Exception) {
            android.util.Log.e("SecureItemTile", "Failed to load thumb for blobId=${item.blobId}", e)
            bitmap = null
            gifBytes = null
        } finally {
            loading = false
        }
    }

    Box(
        modifier = Modifier
            // Fill the JustifiedGrid cell (which is already aspect-sized) instead
            // of forcing a square — a square inside a wide/tall cell left gaps and
            // made the grid look "scattered". Crop fills it cleanly.
            .fillMaxSize()
            .clip(RoundedCornerShape(4.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center
    ) {
        when {
            loading -> CircularProgressIndicator(
                modifier = Modifier.size(24.dp),
                strokeWidth = 2.dp
            )
            gifBytes != null -> AsyncImage(
                model = ImageRequest.Builder(LocalContext.current)
                    .data(java.nio.ByteBuffer.wrap(gifBytes!!))
                    .build(),
                contentDescription = "Secure photo",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
            bitmap != null -> Image(
                bitmap = bitmap!!.asImageBitmap(),
                contentDescription = "Secure photo",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
            else -> {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(
                        painter = painterResource(R.drawable.ic_locks),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(24.dp)
                    )
                    Text(
                        "Encrypted",
                        fontSize = 10.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }

        // ── Subtype / media badges (mirror the main gallery tiles) ──────────
        val sub = item.photoSubtype
        val durLabel = item.durationSecs?.let { d ->
            val m = (d / 60).toInt(); val s = (d % 60).toInt(); "$m:${s.toString().padStart(2, '0')}"
        }
        when (item.mediaType) {
            "video" -> SecureTileBadge("▶" + (durLabel?.let { " $it" } ?: ""), Alignment.BottomStart)
            "gif" -> SecureTileBadge("GIF", Alignment.BottomStart)
            "audio" -> SecureTileBadge("♫" + (durLabel?.let { " $it" } ?: ""), Alignment.BottomStart)
        }
        val topLabel = when {
            sub == "equirectangular" -> "360°"
            sub == "panorama" -> "PANO"
            sub == "motion" -> "LIVE"
            burstCount > 1 -> "BURST $burstCount"
            !item.burstId.isNullOrEmpty() -> "BURST"
            else -> null
        }
        if (topLabel != null) SecureTileBadge(topLabel, Alignment.TopStart, bold = true)
    }
}

/** Small translucent badge used on secure tiles (matches the gallery style). */
@Composable
private fun BoxScope.SecureTileBadge(
    text: String,
    alignment: Alignment,
    bold: Boolean = false
) {
    Surface(
        modifier = Modifier.align(alignment).padding(4.dp),
        shape = MaterialTheme.shapes.extraSmall,
        color = Color.Black.copy(alpha = 0.6f)
    ) {
        Text(
            text = text,
            color = Color.White,
            fontSize = if (bold) 9.sp else 10.sp,
            fontWeight = if (bold) FontWeight.Bold else FontWeight.Normal,
            modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp)
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Photo Thumbnail helper
// ─────────────────────────────────────────────────────────────────────────────

@Composable
internal fun PhotoThumbnail(photo: PhotoEntity) {
    val isGif = photo.mediaType == "gif"
    when {
        isGif && photo.localPath != null -> {
            AsyncImage(
                model = ImageRequest.Builder(LocalContext.current)
                    .data(android.net.Uri.parse(photo.localPath))
                    .build(),
                contentDescription = photo.filename,
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
        }
        photo.thumbnailPath != null -> {
            val bitmap = remember(photo.thumbnailPath) {
                try { BitmapFactory.decodeFile(photo.thumbnailPath) } catch (_: Exception) { null }
            }
            bitmap?.let {
                Image(
                    bitmap = it.asImageBitmap(),
                    contentDescription = photo.filename,
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Crop
                )
            }
        }
        else -> {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .background(MaterialTheme.colorScheme.surfaceVariant),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    photo.filename.take(6),
                    fontSize = 9.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center
                )
            }
        }
    }
}
