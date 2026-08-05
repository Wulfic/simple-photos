package com.simplephotos.ui.screens.securegallery

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale

/**
 * Non-destructive crop/edit display for secure items (#31).
 *
 * Secure items carry their own `crop_metadata` (same JSON shape as a regular
 * photo). This applies it at draw time exactly like the main gallery tile does
 * (see GalleryScreen.kt): the whole image is drawn oversized so its crop
 * sub-rect fills the cell, then offset/rotated so that sub-rect lands at the
 * origin — the parent's clip discards the overflow. No re-decode of the
 * encrypted blob, so it works for photos and GIFs alike.
 */
data class SecureTileCrop(
    val modifier: Modifier,
    val scale: ContentScale,
    val colorFilter: ColorFilter?,
)

/** Parsed crop rect + rotation, used both for the tile transform and grid aspect. */
private data class ParsedCrop(
    val x: Float, val y: Float, val w: Float, val h: Float,
    val rot: Int, val brightness: Float,
) {
    val cropped: Boolean get() = w < 0.999f || h < 0.999f
}

private fun parseCrop(cropMetadata: String?): ParsedCrop? {
    if (cropMetadata.isNullOrEmpty()) return null
    return try {
        val j = org.json.JSONObject(cropMetadata)
        val w = j.optDouble("width", 1.0).toFloat().let { if (it <= 0f) 1f else it }
        val h = j.optDouble("height", 1.0).toFloat().let { if (it <= 0f) 1f else it }
        ParsedCrop(
            x = j.optDouble("x", 0.0).toFloat(),
            y = j.optDouble("y", 0.0).toFloat(),
            w = w,
            h = h,
            rot = ((j.optInt("rotate", 0)) % 360 + 360) % 360,
            brightness = j.optDouble("brightness", 0.0).toFloat(),
        )
    } catch (_: Exception) {
        null
    }
}

/**
 * Crop-effective aspect ratio for laying out a secure tile (mirrors web
 * getEffectiveAspectRatio). The crop fractions are in the ROTATED frame, so we
 * swap the stored dims into that frame before applying them.
 */
fun secureEffectiveAspect(width: Int?, height: Int?, cropMetadata: String?): Float {
    val w0 = width ?: 0
    val h0 = height ?: 0
    if (w0 <= 0 || h0 <= 0) return 1f
    val crop = parseCrop(cropMetadata) ?: return w0.toFloat() / h0.toFloat()
    var w = w0.toFloat()
    var h = h0.toFloat()
    if (crop.rot % 180 != 0) { val t = w; w = h; h = t }
    w *= crop.w
    h *= crop.h
    return if (h > 0f) w / h else 1f
}

/**
 * Compute the draw-time transform that makes an image show only its crop
 * sub-rect, filling the (crop-effective-aspect) cell. Returns a plain cover
 * modifier when there is no crop.
 */
@Composable
fun rememberSecureTileCrop(cropMetadata: String?): SecureTileCrop {
    val crop = remember(cropMetadata) { parseCrop(cropMetadata) }
    return remember(crop) {
        val brightnessFilter = crop?.takeIf { it.brightness != 0f }?.let {
            val b = 1f + it.brightness / 100f
            ColorFilter.colorMatrix(ColorMatrix().apply { setToScale(b, b, b, 1f) })
        }
        when {
            crop == null || (!crop.cropped && crop.rot == 0) ->
                SecureTileCrop(Modifier, ContentScale.Crop, brightnessFilter)

            crop.cropped && crop.rot == 0 -> SecureTileCrop(
                Modifier.graphicsLayer {
                    transformOrigin = TransformOrigin(0f, 0f)
                    scaleX = 1f / crop.w
                    scaleY = 1f / crop.h
                    translationX = -(crop.x / crop.w) * size.width
                    translationY = -(crop.y / crop.h) * size.height
                },
                ContentScale.FillBounds,
                brightnessFilter,
            )

            crop.cropped -> {
                val swapped = crop.rot == 90 || crop.rot == 270
                SecureTileCrop(
                    Modifier.graphicsLayer {
                        rotationZ = crop.rot.toFloat()
                        transformOrigin = TransformOrigin(0.5f, 0.5f)
                        scaleX = if (swapped) 1f / crop.h else 1f / crop.w
                        scaleY = if (swapped) 1f / crop.w else 1f / crop.h
                    },
                    ContentScale.FillBounds,
                    brightnessFilter,
                )
            }

            else -> // uncropped, rotated full-frame
                SecureTileCrop(
                    Modifier.graphicsLayer { rotationZ = crop.rot.toFloat() },
                    ContentScale.Crop,
                    brightnessFilter,
                )
        }
    }
}
