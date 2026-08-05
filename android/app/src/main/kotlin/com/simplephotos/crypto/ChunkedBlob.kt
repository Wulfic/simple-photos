/**
 * Format-aware decoding of photo blobs across both container layouts.
 */
package com.simplephotos.crypto

import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.security.DigestOutputStream
import java.security.MessageDigest
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

    /** `blobs.blob_format` value for the chunked container. Matches server `FORMAT_V2`. */
    const val FORMAT_V2: Int = 2

    /**
     * Plaintext bytes per chunk frame (4 MiB) — must match server `CHUNK_SIZE` so
     * the on-disk framing is identical. Bounds peak encrypt/decrypt heap to ~one
     * chunk regardless of file size.
     */
    const val CHUNK_SIZE: Int = 4 * 1024 * 1024

    /**
     * Sources at or above this size are uploaded with the chunked v2 container.
     * Smaller media keeps the v1 monolithic envelope (peak ≈ 5× size is safe
     * there). Matches server `CHUNKED_THRESHOLD_BYTES` (32 MiB).
     */
    const val CHUNKED_THRESHOLD_BYTES: Long = 32L * 1024 * 1024

    /**
     * Per-frame AES-GCM overhead: 12-byte nonce prefix + 16-byte GCM tag. Matches
     * server `FRAME_OVERHEAD`. Every frame's ciphertext is its plaintext + this.
     */
    const val FRAME_OVERHEAD: Int = 12 + 16

    /**
     * On-disk size of a *full* chunk frame: 4-byte big-endian length prefix +
     * nonce + [CHUNK_SIZE] plaintext + tag. Matches server `FULL_FRAME_DISK_SIZE`.
     * `Long` because a blob can exceed 2 GiB — the whole reason v2 exists.
     */
    const val FULL_FRAME_DISK_SIZE: Long = 4L + FRAME_OVERHEAD + CHUNK_SIZE

    /**
     * Byte offset of the first chunk frame, given the encrypted metadata-frame
     * length read from the header. Layout: `MAGIC(8) + metaLen:u32(4) + encMeta`.
     * Matches server `read_header_chunks_start`.
     */
    fun chunksStart(metaLen: Int): Long = MAGIC_SIZE.toLong() + 4L + metaLen

    /**
     * Total *plaintext* length of a v2 chunked blob, derived purely from the
     * encrypted total size and the frame geometry — no decryption, no full-file
     * walk. The server encoder writes exactly [CHUNK_SIZE] plaintext per non-final
     * frame, so the layout is deterministic: every full frame occupies
     * [FULL_FRAME_DISK_SIZE] on disk and only the trailing frame is short. Mirrors
     * the sum that server `plaintext_len_from_file` computes by walking frames.
     */
    fun plaintextTotalOf(encryptedTotal: Long, chunksStart: Long): Long {
        val dataRegion = encryptedTotal - chunksStart
        require(dataRegion >= 0) { "chunked blob ($encryptedTotal) smaller than header ($chunksStart)" }
        val fullBlocks = dataRegion / FULL_FRAME_DISK_SIZE
        val remainder = dataRegion % FULL_FRAME_DISK_SIZE
        val lastPlain = if (remainder == 0L) {
            0L
        } else {
            val lp = remainder - 4L - FRAME_OVERHEAD
            require(lp > 0L) { "malformed trailing chunk frame ($remainder bytes)" }
            lp
        }
        return fullBlocks * CHUNK_SIZE + lastPlain
    }

    /**
     * What the caller needs to build the upload request and `blobs` row after a
     * streaming encrypt, without ever holding the whole file in memory.
     */
    data class ChunkedEncryptResult(
        /** Total bytes of the encrypted container written to [encryptStreamToFile]'s `dst`. */
        val blobSize: Long,
        /**
         * Short hash of the **original** (pre-encryption) source bytes — the
         * `X-Content-Hash` for cross-platform dedup. Matches the server's
         * `compute_photo_hash`: first 6 bytes of SHA-256, as 12 hex chars.
         */
        val contentHashHex: String,
        /**
         * Full SHA-256 (64 hex chars) of the **entire encrypted container** — the
         * `X-Client-Hash` the upload handler re-computes over the streamed body to
         * verify integrity.
         */
        val clientHashHex: String,
    )

    /**
     * Stream-encrypt [source] into a **v2 chunked** container at [dst] with
     * bounded heap (~one [CHUNK_SIZE] chunk at a time), mirroring the server
     * format byte-for-byte: `MAGIC` + `metaLen:u32 BE` + AES-GCM([metadataJson])
     * + repeated (`frameLen:u32 BE` + AES-GCM(≤CHUNK_SIZE plaintext)).
     *
     * In the single pass it derives both digests the uploader needs (see
     * [ChunkedEncryptResult]): the content hash over the raw source bytes and the
     * client hash over every byte written to disk.
     */
    fun encryptStreamToFile(
        crypto: Cryptor,
        source: InputStream,
        dst: File,
        metadataJson: ByteArray,
    ): ChunkedEncryptResult {
        val containerDigest = MessageDigest.getInstance("SHA-256")
        val sourceDigest = MessageDigest.getInstance("SHA-256")

        DataOutputStream(DigestOutputStream(dst.outputStream().buffered(), containerDigest)).use { out ->
            out.write(MAGIC)

            // Metadata frame: the v1 envelope minus the media `data`, encrypted as
            // one AES-GCM message. Small, so it never threatens the heap budget.
            val encMeta = crypto.encrypt(metadataJson)
            out.writeInt(encMeta.size) // big-endian, matches server to_be_bytes()
            out.write(encMeta)

            val buf = ByteArray(CHUNK_SIZE)
            while (true) {
                val n = fillBuffer(source, buf)
                if (n <= 0) break
                sourceDigest.update(buf, 0, n)
                val frame = crypto.encrypt(if (n == buf.size) buf else buf.copyOf(n))
                out.writeInt(frame.size)
                out.write(frame)
            }
        }

        return ChunkedEncryptResult(
            blobSize = dst.length(),
            contentHashHex = sourceDigest.digest().copyOf(6).toHex(),
            clientHashHex = containerDigest.digest().toHex(),
        )
    }

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
    fun decryptPhotoBlobBytes(crypto: Cryptor, enc: ByteArray): ByteArray {
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
    fun decryptChunkedStreamToFile(crypto: Cryptor, input: InputStream, outputFile: File) {
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

    /**
     * Read from [input] until [buf] is full or the stream ends, returning the
     * number of bytes read. A single `read` may return fewer bytes than asked
     * (common for file/network streams), so we loop to pack each chunk fully —
     * otherwise frames would be sized by the OS read granularity, not [CHUNK_SIZE].
     */
    private fun fillBuffer(input: InputStream, buf: ByteArray): Int {
        var off = 0
        while (off < buf.size) {
            val n = input.read(buf, off, buf.size - off)
            if (n < 0) break
            off += n
        }
        return off
    }

    private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }

    internal fun readU32BE(b: ByteArray, off: Int): Int {
        require(off + 4 <= b.size) { "truncated chunked blob (length prefix)" }
        return ((b[off].toInt() and 0xFF) shl 24) or
            ((b[off + 1].toInt() and 0xFF) shl 16) or
            ((b[off + 2].toInt() and 0xFF) shl 8) or
            (b[off + 3].toInt() and 0xFF)
    }
}

/**
 * Random-access **plaintext** reader over a v2 chunked blob that fetches and
 * decrypts only the ~4 MiB frame(s) overlapping the requested position — never
 * the whole file. This is what lets a video start (and seek) without first
 * downloading + decrypting a multi-gigabyte blob (issue #17: large videos
 * failing / small videos buffering slowly).
 *
 * Seek is O(1) frame arithmetic, mirroring the server's
 * `decrypt_chunked_range_from_file`: plaintext byte `p` lives in frame
 * `p / CHUNK_SIZE`, whose on-disk length-prefix sits at
 * `chunksStart + frameIdx * FULL_FRAME_DISK_SIZE`.
 *
 * Deliberately free of Android and network types — [fetchBlock] (a blocking
 * byte-range fetch) and [decryptFrame] (single AES-GCM frame) are injected, so
 * the seek/slice/EOF arithmetic is unit-testable on the plain JVM with an
 * in-memory container. One decrypted frame is cached so sequential reads within
 * a frame don't re-fetch.
 */
class ChunkedRandomReader(
    private val chunksStart: Long,
    /** Total plaintext length — see [ChunkedBlob.plaintextTotalOf]. */
    val plaintextTotal: Long,
    /** Fetch exactly the bytes `[diskOffset, diskOffset+len)` (fewer at EOF). Blocking. */
    private val fetchBlock: (diskOffset: Long, len: Long) -> ByteArray,
    /** Decrypt one AES-GCM frame (`[nonce][ciphertext+tag]`) to plaintext. */
    private val decryptFrame: (ByteArray) -> ByteArray,
) {
    private var cachedIdx: Long = -1
    private var cached: ByteArray = ByteArray(0)

    /**
     * Copy up to [length] plaintext bytes starting at [position] into [dst] at
     * [dstOffset]; returns the number copied (always ≥ 1 when in range) or `-1`
     * at/after end-of-media. Serves from a single frame per call — the caller
     * loops for more, exactly like an [okhttp3.ResponseBody] stream read.
     */
    fun readInto(position: Long, dst: ByteArray, dstOffset: Int, length: Int): Int {
        require(position >= 0) { "negative position $position" }
        if (position >= plaintextTotal || length <= 0) return -1
        val idx = position / ChunkedBlob.CHUNK_SIZE
        if (idx != cachedIdx) {
            val diskOff = chunksStart + idx * ChunkedBlob.FULL_FRAME_DISK_SIZE
            val block = fetchBlock(diskOff, ChunkedBlob.FULL_FRAME_DISK_SIZE)
            if (block.size < 4) throw IOException("chunk $idx: short length prefix (${block.size}B)")
            val frameLen = ChunkedBlob.readU32BE(block, 0)
            if (block.size < 4 + frameLen) {
                throw IOException("chunk $idx: truncated frame (have ${block.size}, need ${4 + frameLen})")
            }
            cached = decryptFrame(block.copyOfRange(4, 4 + frameLen))
            cachedIdx = idx
        }
        val frameStart = idx * ChunkedBlob.CHUNK_SIZE
        val within = (position - frameStart).toInt()
        val avail = cached.size - within
        if (avail <= 0) return -1
        val n = minOf(length.toLong(), avail.toLong(), plaintextTotal - position).toInt()
        System.arraycopy(cached, within, dst, dstOffset, n)
        return n
    }
}
