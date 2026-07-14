/**
 * Content-based media-type detection — the device-side mirror of the server's
 * `server/src/media.rs` GIF rescue.
 *
 * Extension- and MediaStore-MIME-based classification silently misses GIFs that
 * were renamed (`funny.jpg`), delivered with a generic MIME, or exported oddly
 * by Google Takeout: they get tagged `photo` and vanish from the GIF smart
 * album (issue #14). When we hold the plaintext bytes, the magic-byte signature
 * is authoritative. Keep in lockstep with:
 *   - server/src/media.rs :: is_gif_header / gif_override
 */
package com.simplephotos.data.media

object MediaTypeDetector {

    /** The 6-byte GIF signature (`GIF87a` / `GIF89a`) that opens every GIF. */
    private val GIF87A = "GIF87a".toByteArray(Charsets.US_ASCII)
    private val GIF89A = "GIF89a".toByteArray(Charsets.US_ASCII)

    /** True when [bytes] begins with a GIF magic signature. */
    fun isGif(bytes: ByteArray): Boolean =
        bytes.startsWith(GIF87A) || bytes.startsWith(GIF89A)

    /**
     * Given an extension/MIME-derived [mediaType] and the file's leading
     * [bytes], return the corrected media type. Only ever upgrades an image to
     * `"gif"` — never reclassifies `video`/`audio` off a stray GIF signature.
     */
    fun rescueGif(mediaType: String, bytes: ByteArray): String =
        if (mediaType != "gif" && mediaType != "video" && mediaType != "audio" && isGif(bytes)) {
            "gif"
        } else {
            mediaType
        }

    private fun ByteArray.startsWith(prefix: ByteArray): Boolean {
        if (size < prefix.size) return false
        for (i in prefix.indices) if (this[i] != prefix[i]) return false
        return true
    }
}
