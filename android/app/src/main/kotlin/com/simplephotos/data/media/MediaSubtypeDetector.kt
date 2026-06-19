/**
 * Client-side detection of special photo subtypes (motion / panorama / 360° /
 * HDR / burst) from the raw, plaintext bytes of a captured photo.
 *
 * ## Why this lives on the device
 *
 * The Android backup path is **end-to-end encrypted**: each photo is encrypted
 * with the user's key and uploaded as an opaque blob, then registered via
 * `register-encrypted`. The server only ever sees ciphertext, so the
 * battle-tested XMP scanner in the Rust server (`metadata::extract_xmp_subtype`)
 * can never run on these photos. If we don't detect the subtype here — while we
 * still hold the plaintext bytes — motion photos and bursts arrive on the
 * server with `photo_subtype = NULL` and render as ordinary stills (no LIVE
 * badge, no burst stacking).
 *
 * This is a faithful Kotlin port of the server's detection so the two ingest
 * paths (web plaintext upload vs. Android encrypted backup) classify photos
 * identically. Keep it in sync with:
 *   - server/src/photos/metadata.rs :: extract_xmp_subtype
 *   - server/src/photos/metadata.rs :: apply_aspect_subtype_fallback
 *   - server/src/photos/motion.rs  :: find_samsung_motion_offset
 *
 * ## Burst note
 *
 * Pixel bursts carry `GCamera:BurstID` in XMP and are grouped here. Samsung
 * (and most other) bursts have **no** BurstID — those are grouped server-side
 * by timestamp proximity (`burst::detect_bursts_for_user`), which is why the
 * upload path also sends `camera_model`. We deliberately do NOT assign a
 * timestamp-based burst id on-device: grouping needs all frames at once, and
 * the server already does it correctly.
 */
package com.simplephotos.data.media

/**
 * Result of subtype detection. Mirrors the server's `SubtypeInfo`.
 *
 * @param photoSubtype one of "motion", "panorama", "equirectangular", "hdr",
 *        "burst", or null for an ordinary photo.
 * @param burstId burst group id when the file carries an XMP `BurstID`
 *        (orthogonal to [photoSubtype] — a burst frame can also be a motion
 *        photo / HDR), else null.
 */
data class MediaSubtype(
    val photoSubtype: String? = null,
    val burstId: String? = null,
)

object MediaSubtypeDetector {

    /** Samsung SEF trailer marker; the embedded MP4 starts right after it. */
    private val SAMSUNG_MOTION_MARKER = "MotionPhoto_Data".toByteArray(Charsets.US_ASCII)

    /** XMP packets sit in the first few KB; scan a generous prefix. */
    private const val XMP_SCAN_PREFIX_BYTES = 256 * 1024

    /** Bound the trailing SEF marker scan so a huge file can't stall us. */
    private const val SAMSUNG_TRAILER_SCAN_BYTES = 16 * 1024 * 1024

    /**
     * Detect the subtype of an image from its raw bytes.
     *
     * Only meaningful for still images (JPEG/HEIC). Callers should skip videos
     * and GIFs. [width]/[height] feed the aspect-ratio panorama fallback; pass
     * 0 when unknown to disable it.
     */
    fun detect(data: ByteArray, width: Int = 0, height: Int = 0): MediaSubtype {
        // Decode the XMP prefix as text once. XMP is ASCII/UTF-8; lossy decode
        // is fine since we only look for marker substrings.
        val prefixLen = minOf(data.size, XMP_SCAN_PREFIX_BYTES)
        val text = String(data, 0, prefixLen, Charsets.ISO_8859_1)

        // ── Burst ID (orthogonal to subtype) ────────────────────────────────
        val burstId = xmpStrAttr(text, "BurstID")

        // ── Motion photo ────────────────────────────────────────────────────
        // Pixel/Google: GCamera:MotionPhoto / Camera:MicroVideo flag in XMP.
        // Samsung: typically no XMP flag — the giveaway is the trailing
        // `MotionPhoto_Data` SEF marker, so fall back to scanning for it.
        val motionFlag = xmpStrAttr(text, "MicroVideo") ?: xmpStrAttr(text, "MotionPhoto")
        val isMotionByXmp = motionFlag?.trim()?.let {
            it.isNotEmpty() && !it.equals("0", true) && !it.equals("false", true)
        } ?: false

        if (isMotionByXmp || hasSamsungMotionTrailer(data)) {
            return MediaSubtype("motion", burstId)
        }

        // ── Panorama / 360° ──────────────────────────────────────────────────
        val proj = xmpStrAttr(text, "ProjectionType")?.lowercase()
        when (proj) {
            "equirectangular" -> return MediaSubtype("equirectangular", burstId)
            "cylindrical", "fisheye" -> return MediaSubtype("panorama", burstId)
        }

        // ── Burst subtype (before HDR — Pixel bursts are routinely Ultra HDR) ─
        if (burstId != null) {
            return MediaSubtype("burst", burstId)
        }

        // ── HDR Gainmap (Ultra HDR) ──────────────────────────────────────────
        if (text.contains("hdrgm:Version") || text.contains("HDRGainMap")) {
            return MediaSubtype("hdr", burstId)
        }

        // ── Aspect-ratio fallback (XMP-less panoramas / 360°) ────────────────
        aspectFallback(width, height)?.let { return MediaSubtype(it, burstId) }

        return MediaSubtype(null, burstId)
    }

    /**
     * True when the file ends with a Samsung motion-photo SEF trailer. Mirrors
     * `motion::find_samsung_motion_offset`: scan the last 16 MB for the marker
     * and require at least some bytes to follow it (the embedded MP4).
     */
    private fun hasSamsungMotionTrailer(data: ByteArray): Boolean {
        val start = maxOf(0, data.size - SAMSUNG_TRAILER_SCAN_BYTES)
        val pos = lastIndexOf(data, SAMSUNG_MOTION_MARKER, start)
        if (pos < 0) return false
        val mp4Start = pos + SAMSUNG_MOTION_MARKER.size
        return mp4Start < data.size
    }

    /**
     * Aspect-ratio panorama fallback. Mirrors
     * `metadata::apply_aspect_subtype_fallback` exactly (thresholds included).
     */
    private fun aspectFallback(width: Int, height: Int): String? {
        if (width <= 0 || height <= 0) return null
        if (maxOf(width, height) < 2048) return null
        val w = width.toDouble()
        val h = height.toDouble()
        val aspect = w / h
        val isHorizontalPano = aspect >= 2.0
        val isVerticalPano = (1.0 / aspect) >= 2.5
        if (!isHorizontalPano && !isVerticalPano) return null
        return if (isHorizontalPano && aspect in 1.97..2.03 && width >= 4000) {
            "equirectangular"
        } else {
            "panorama"
        }
    }

    // ── XMP attribute helpers (tolerate either quote style, any prefix) ───────

    /** Extract a string attribute value: `AttrName="v"` or `AttrName='v'`. */
    private fun xmpStrAttr(text: String, attrName: String): String? {
        for (quote in charArrayOf('"', '\'')) {
            val pattern = "$attrName=$quote"
            val pos = text.indexOf(pattern)
            if (pos >= 0) {
                val start = pos + pattern.length
                val end = text.indexOf(quote, start)
                if (end > start) {
                    val v = text.substring(start, end).trim()
                    if (v.isNotEmpty()) return v
                }
            }
        }
        return null
    }

    /** Last index of [needle] in [haystack] at or after [from]. */
    private fun lastIndexOf(haystack: ByteArray, needle: ByteArray, from: Int): Int {
        if (needle.isEmpty() || haystack.size < needle.size) return -1
        var i = haystack.size - needle.size
        while (i >= from) {
            var match = true
            for (j in needle.indices) {
                if (haystack[i + j] != needle[j]) { match = false; break }
            }
            if (match) return i
            i--
        }
        return -1
    }
}
