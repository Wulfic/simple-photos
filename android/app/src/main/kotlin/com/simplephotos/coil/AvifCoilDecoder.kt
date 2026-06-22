package com.simplephotos.coil

import android.graphics.Bitmap
import android.graphics.drawable.BitmapDrawable
import android.util.Log
import coil.ImageLoader
import coil.decode.DecodeResult
import coil.decode.DecodeUtils
import coil.decode.Decoder
import coil.fetch.SourceResult
import coil.request.Options
import coil.size.pxOrElse
import okio.BufferedSource
import okio.ByteString.Companion.encodeUtf8
import org.aomedia.avif.android.AvifDecoder
import java.nio.ByteBuffer
import kotlin.math.roundToInt

/**
 * Coil [Decoder] for still AVIF / HEIC images, backed by the bundled libavif +
 * dav1d ([org.aomedia.avif.android.AvifDecoder]).
 *
 * Why this exists: the Android platform decoders (BitmapFactory and
 * ImageDecoder) return `getPixels failed with error invalid input` for some
 * perfectly standard AVIFs on certain devices — notably large equirectangular
 * 360° panoramas on Samsung — which rendered as a pure black image. Coil's own
 * `ImageDecoderDecoder` only handles *animated* formats, so still AVIF fell
 * through to BitmapFactory and failed. Decoding in software here makes AVIF
 * render everywhere the app uses Coil (gallery grid, viewer, secure gallery,
 * the 360 sphere), matching the web client which decodes AVIF via dav1d in the
 * browser.
 */
class AvifCoilDecoder(
    private val source: SourceResult,
    private val options: Options,
) : Decoder {

    override suspend fun decode(): DecodeResult? {
        val bytes = source.source.source().use { it.readByteArray() }
        if (bytes.size < 12) return null

        // libavif's JNI reads through a direct ByteBuffer with position() == 0.
        val buffer = ByteBuffer.allocateDirect(bytes.size)
        buffer.put(bytes)
        buffer.position(0)

        val info = AvifDecoder.Info()
        if (!AvifDecoder.getInfo(buffer, buffer.remaining(), info) ||
            info.width <= 0 || info.height <= 0
        ) {
            Log.w(TAG, "getInfo failed or zero-size for ${source.source}")
            return null
        }

        // Honour Coil's requested size (keeps thumbnails small) without upscaling,
        // then hard-cap the longest edge so a huge panorama stays within budget.
        // AvifDecoder.decode() scales into whatever bitmap size we give it.
        val dstWidth = options.size.width.pxOrElse { info.width }
        val dstHeight = options.size.height.pxOrElse { info.height }
        var multiplier = DecodeUtils.computeSizeMultiplier(
            srcWidth = info.width,
            srcHeight = info.height,
            dstWidth = dstWidth,
            dstHeight = dstHeight,
            scale = options.scale,
        ).coerceAtMost(1.0)
        var targetW = (info.width * multiplier).roundToInt().coerceAtLeast(1)
        var targetH = (info.height * multiplier).roundToInt().coerceAtLeast(1)
        val longest = maxOf(targetW, targetH)
        if (longest > MAX_DECODE_PX) {
            val cap = MAX_DECODE_PX.toDouble() / longest
            targetW = (targetW * cap).roundToInt().coerceAtLeast(1)
            targetH = (targetH * cap).roundToInt().coerceAtLeast(1)
            multiplier *= cap
        }

        val bitmap = Bitmap.createBitmap(targetW, targetH, Bitmap.Config.ARGB_8888)
        buffer.position(0)
        if (!AvifDecoder.decode(buffer, buffer.remaining(), bitmap)) {
            bitmap.recycle()
            Log.w(TAG, "libavif decode failed (${info.width}x${info.height} → ${targetW}x$targetH)")
            return null
        }
        return DecodeResult(
            drawable = BitmapDrawable(options.context.resources, bitmap),
            isSampled = multiplier < 1.0,
        )
    }

    class Factory : Decoder.Factory {
        override fun create(result: SourceResult, options: Options, imageLoader: ImageLoader): Decoder? {
            if (!isSupported(result.source.source())) return null
            return AvifCoilDecoder(result, options)
        }

        /** ISO-BMFF `ftyp` at offset 4 followed by a still AVIF/HEIC brand at offset 8. */
        private fun isSupported(source: BufferedSource): Boolean {
            if (!source.rangeEquals(4, FTYP)) return false
            return BRANDS.any { source.rangeEquals(8, it) }
        }
    }

    private companion object {
        const val TAG = "AvifCoilDecoder"
        const val MAX_DECODE_PX = 4096
        val FTYP = "ftyp".encodeUtf8()
        val BRANDS = listOf("avif", "avis", "heic", "heix", "mif1", "msf1", "hevc", "hevx")
            .map { it.encodeUtf8() }
    }
}
