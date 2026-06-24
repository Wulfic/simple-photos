/**
 * Shared photo-tile overlays — the small badges that several gallery tiles
 * (MediaTile, AlbumPhotoTile, TrashTile) drew identically on top of their
 * thumbnail Box.
 *
 * Only the genuinely byte-identical overlays live here: the green selection
 * circle and the cloud-backup badge. The per-tile media-type / duration /
 * subtype / burst badges are intentionally NOT shared — they diverge in
 * alignment, shape, and text between tiles (and that styling is device-verified),
 * so each tile keeps its own.
 *
 * These are `BoxScope` extensions so they can use `Modifier.align` against the
 * tile's root Box, exactly as the inline versions did.
 */
package com.simplephotos.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import coil.compose.AsyncImage
import com.simplephotos.R

/** The green used for the active selection circle (matches the web accent). */
private val SelectionGreen = Color(0xFF22C55E)

/**
 * Top-right selection indicator: a hollow circle that fills green with a white
 * check when selected. Callers gate this with their own `isSelectionMode` check.
 *
 * Defaults match AlbumPhotoTile / TrashTile (the byte-identical pair); MediaTile
 * passes the smaller [padding] / [size] / [checkSize] it has always used.
 */
@Composable
fun BoxScope.TileSelectionCircle(
    isSelected: Boolean,
    padding: Dp = 6.dp,
    size: Dp = 24.dp,
    checkSize: Dp = 16.dp,
) {
    Box(
        modifier = Modifier
            .align(Alignment.TopEnd)
            .padding(padding)
            .size(size)
            .clip(CircleShape)
            .background(if (isSelected) SelectionGreen else Color.White.copy(alpha = 0.8f))
            .border(
                width = 2.dp,
                color = if (isSelected) SelectionGreen else Color.Gray.copy(alpha = 0.5f),
                shape = CircleShape,
            ),
        contentAlignment = Alignment.Center,
    ) {
        if (isSelected) {
            Icon(
                Icons.Default.Check,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(checkSize),
            )
        }
    }
}

/**
 * Bottom-right cloud icon shown when a photo is backed up to the server and
 * still present on the device. Callers gate this with the sync/localPath check.
 */
@Composable
fun BoxScope.CloudBackupBadge() {
    AsyncImage(
        model = R.drawable.ic_cloud,
        contentDescription = "Backed up to cloud",
        modifier = Modifier
            .align(Alignment.BottomEnd)
            .padding(4.dp)
            .size(18.dp),
    )
}
