/**
 * Format-aware decoding of photo blobs across both container layouts.
 */
package com.simplephotos.crypto

import java.io.DataInputStream
import java.io.File
import java.io.InputStream
import org.json.JSONObject

/**
 * The server stores a photo's media in one of two encrypted containers:
 *
 *  - **v1 (monolithic):** `AES-GCM( {"v":1, ...metadata..., "data": base64(bytes)} )`
 *    — one AES-GCM message with the whole file base64'd inside the JSON.
 *  - **v2 (chunked):** a `SPCHNKB2` container of an encrypted metadata frame
 *    followed by length-prefixed AES-GCM chunk frames (see server
 *    `blobs/chunked.rs`). Used for large videos so neither side holds a
 *    multi-gigabyte payload in memory at once.
 *
 * Format is detected from the downloaded bytes (the v2 magic prefix) — robust
 * even when the `X-Blob-Format` response header is stripped by a proxy.
 */
object ChunkedBlob {
    /** Must match `MAGIC` in server `blobs/chunked.rs`. */
    private val MAGIC = "SPCHNKB2".toByteArray(Charsets.US_ASCII)

    /** Number of leading bytes needed to detect the format. */
    val MAGIC_SIZE: Int = MAGIC.size

    fun isChunked(enc: ByteArray): Boolean = isChunked(enc, enc.size)

    fun isChunked(head: ByteArray, len: Int): Boolean {
        if (len < MAGIC.size) return false
        for (i in MAGIC.indices) if (head[i] != MAGIC[i]) return false
        return true
    }

    /**
     * Decrypt a photo blob (either format) fully into media bytes in memory.
     * Used for photos Coil renders from a `ByteArray`. Videos use the streaming
     * [decryptChunkedStreamToFile] path instead.
     */
    fun decryptPhotoBlobBytes(crypto: CryptoManager, enc: ByteArray): ByteArray {
        if (!isChunked(enc)) {
            // v1 monolithic envelope.
            val decrypted = crypto.decrypt(enc)
            val payload = JSONObject(String(decrypted, Charsets.UTF_8))
            val dataB64 = payload.getString("data")
            return android.util.Base64.decode(dataB64, android.util.Base64.NO_WRAP)
        }
        // v2 chunked: skip magic + metadata frame, concat decrypted chunk frames.
        var cur = MAGIC.size
        val metaLen = readU32BE(enc, cur); cur += 4
        require(cur + metaLen <= enc.size) { "truncated chunked blob (metadata)" }
        cur += metaLen
        val out = java.io.ByteArrayOutputStream()
        while (cur < enc.size) {
            val frameLen = readU32BE(enc, cur); cur += 4
            require(cur + frameLen <= enc.size) { "truncated chunked blob (frame body)" }
            out.write(crypto.decrypt(enc.copyOfRange(cur, cur + frameLen)))
            cur += frameLen
        }
        return out.toByteArray()
    }

    /**
     * Stream-decrypt a **v2 chunked** blob to [outputFile] with bounded heap:
     * each ~4 MiB chunk frame is read, decrypted, and appended in turn, so a
     * multi-gigabyte video never lives in the Java heap. [input] must be
     * positioned at the start of the blob (the magic is consumed here).
     */
    fun decryptChunkedStreamToFile(crypto: CryptoManager, input: InputStream, outputFile: File) {
        val din = DataInputStream(input.buffered())
        val magic = ByteArray(MAGIC.size)
        din.readFully(magic) // already format-detected by the caller; consume it
        val metaLen = din.readInt() // big-endian, matches server to_be_bytes()
        // Skip the metadata frame (not needed for the media bytes).
        var toSkip = metaLen
        val discard = ByteArray(minOf(metaLen, 64 * 1024).coerceAtLeast(1))
        while (toSkip > 0) {
            val n = din.read(discard, 0, minOf(toSkip, discard.size))
            if (n < 0) throw java.io.EOFException("truncated chunked blob (metadata)")
            toSkip -= n
        }
        outputFile.outputStream().buffered().use { out ->
            while (true) {
                val frameLen = try {
                    din.readInt()
                } catch (_: java.io.EOFException) {
                    break // clean end of frames
                }
                val frame = ByteArray(frameLen)
                din.readFully(frame)
                out.write(crypto.decrypt(frame))
            }
        }
    }

    private fun readU32BE(b: ByteArray, off: Int): Int {
        require(off + 4 <= b.size) { "truncated chunked blob (length prefix)" }
        return ((b[off].toInt() and 0xFF) shl 24) or
            ((b[off + 1].toInt() and 0xFF) shl 16) or
            ((b[off + 2].toInt() and 0xFF) shl 8) or
            (b[off + 3].toInt() and 0xFF)
    }
}
