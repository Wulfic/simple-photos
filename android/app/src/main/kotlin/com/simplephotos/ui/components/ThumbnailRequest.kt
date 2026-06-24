/**
 * Shared Coil [ImageRequest] builder for thumbnail / photo loads.
 *
 * Replaces the `ImageRequest.Builder(context).data(..).crossfade(true)[.size(..)]
 * .build()` boilerplate that was hand-rolled across ~11 screens (gallery, album
 * list/detail, search, trash, library, secure tiles, and the photo viewers).
 *
 * Auth is intentionally NOT set here: the `Authorization: Bearer` header is
 * injected globally by the OkHttp interceptor in NetworkModule, which backs the
 * shared Coil ImageLoader — so a plain data load is already authenticated.
 */
package com.simplephotos.ui.components

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import coil.request.ImageRequest

/**
 * Build (and remember) a Coil [ImageRequest] with the app's standard options.
 *
 * @param data          the Coil model: URL string, [java.io.File], Uri,
 *                      ByteArray, ByteBuffer, etc.
 * @param size          optional decode-size cap in px. Omit (null) for GIFs and
 *                      other cases that need the full data.
 * @param crossfade     enable the crossfade transition (default true). Pass
 *                      false to match call sites that never set it.
 * @param allowHardware false forces a software Bitmap — required for the capped
 *                      pano / 360 decode path (see MAX_PANO_DECODE_PX).
 */
@Composable
fun rememberThumbnailRequest(
    data: Any?,
    size: Int? = null,
    crossfade: Boolean = true,
    allowHardware: Boolean = true,
): ImageRequest {
    val context = LocalContext.current
    return remember(data, size, crossfade, allowHardware) {
        ImageRequest.Builder(context)
            .data(data)
            .crossfade(crossfade)
            .apply {
                size?.let { size(it) }
                if (!allowHardware) allowHardware(false)
            }
            .build()
    }
}
