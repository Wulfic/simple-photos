/**
 * ExoPlayer [DataSource] that plays encrypted media blobs by fetching and
 * decrypting only the frames the player actually reads — instead of downloading
 * and decrypting the whole file to a temp file before playback could begin.
 *
 * That old whole-file path is exactly why issue #17 happened: a large video had
 * to be fully downloaded + decrypted (multi-gigabyte, cache-filling, slow or
 * OOM) before the first frame, and even a small video waited on the full
 * download before it could start.
 *
 * A `spblob://<blobId>` URI routes here; any other scheme (`file://`,
 * `content://` for local originals / motion trailers) delegates to a standard
 * [DefaultDataSource]. For v2 chunked blobs (≥ 32 MiB — i.e. real videos) reads
 * are O(1) frame arithmetic over HTTP Range requests; v1 monolithic blobs
 * (< 32 MiB) fall back to a one-shot whole-file decrypt held in memory, which is
 * bounded by the v2 threshold.
 */
package com.simplephotos.ui.screens.viewer

import android.content.Context
import android.net.Uri
import androidx.media3.common.C
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.BaseDataSource
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSpec
import androidx.media3.datasource.DefaultDataSource
import com.simplephotos.crypto.ChunkedBlob
import com.simplephotos.crypto.ChunkedRandomReader
import com.simplephotos.data.repository.EncryptedBlobStream
import kotlinx.coroutines.runBlocking
import java.io.IOException

@UnstableApi
class MediaBlobDataSource(
    context: Context,
    private val blobStream: EncryptedBlobStream,
) : BaseDataSource(/* isNetwork = */ true) {

    companion object {
        const val SCHEME = "spblob"

        /** Build a `spblob://<blobId>` URI for the encrypted-blob streaming path. */
        fun uriFor(blobId: String): Uri = Uri.parse("$SCHEME://$blobId")
    }

    // Delegate for non-spblob URIs (local file:// originals, motion trailers, …).
    private val defaultSource: DataSource = DefaultDataSource.Factory(context).createDataSource()
    private var usingDefault = false

    private var uri: Uri? = null

    // Resolved geometry, cached by blob id so a seek (close→open) neither
    // re-probes the header nor — critically for v1 — re-downloads the whole blob.
    private var resolvedBlobId: String? = null
    private var reader: ChunkedRandomReader? = null
    private var v1Plain: ByteArray? = null
    private var plaintextTotal: Long = 0

    // Per-open cursor.
    private var position: Long = 0
    private var bytesRemaining: Long = 0

    override fun open(dataSpec: DataSpec): Long {
        uri = dataSpec.uri
        if (!SCHEME.equals(dataSpec.uri.scheme, ignoreCase = true)) {
            usingDefault = true
            return defaultSource.open(dataSpec)
        }
        usingDefault = false
        transferInitializing(dataSpec)

        val blobId = dataSpec.uri.host?.takeIf { it.isNotEmpty() }
            ?: throw IOException("spblob URI missing blob id: ${dataSpec.uri}")

        if (blobId != resolvedBlobId) {
            resolveGeometry(blobId)
            resolvedBlobId = blobId
        }

        position = dataSpec.position
        bytesRemaining = if (dataSpec.length == C.LENGTH_UNSET.toLong()) {
            plaintextTotal - position
        } else {
            dataSpec.length
        }
        if (bytesRemaining < 0) {
            throw IOException("open position ${dataSpec.position} beyond media length $plaintextTotal")
        }

        transferStarted(dataSpec)
        return bytesRemaining
    }

    /** Probe the header (magic + metadata length) and set up the reader (v2) or
     *  the whole-file plaintext (v1). Blocking — runs on ExoPlayer's loader thread. */
    private fun resolveGeometry(blobId: String) {
        reader = null
        v1Plain = null
        // 12 bytes = MAGIC(8) + metaLen:u32(4); the 206 Content-Range gives the total.
        val probe = runBlocking { blobStream.fetchRange(blobId, 0, 11) }
        val head = probe.bytes
        if (ChunkedBlob.isChunked(head, head.size)) {
            val metaLen = ChunkedBlob.readU32BE(head, ChunkedBlob.MAGIC_SIZE)
            val chunksStart = ChunkedBlob.chunksStart(metaLen)
            plaintextTotal = ChunkedBlob.plaintextTotalOf(probe.encryptedTotal, chunksStart)
            reader = ChunkedRandomReader(
                chunksStart = chunksStart,
                plaintextTotal = plaintextTotal,
                fetchBlock = { off, len ->
                    runBlocking { blobStream.fetchRange(blobId, off, off + len - 1).bytes }
                },
                decryptFrame = { frame -> blobStream.decryptFrame(frame) },
            )
        } else {
            // v1 monolithic (< 32 MiB) — no per-frame seeking; decrypt once.
            val plain = runBlocking { blobStream.fetchWholePlaintext(blobId) }
            v1Plain = plain
            plaintextTotal = plain.size.toLong()
        }
    }

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
        if (usingDefault) return defaultSource.read(buffer, offset, length)
        if (length == 0) return 0
        if (bytesRemaining == 0L) return C.RESULT_END_OF_INPUT

        val toRead = minOf(length.toLong(), bytesRemaining).toInt()
        val n = readPlaintext(position, buffer, offset, toRead)
        if (n < 0) return C.RESULT_END_OF_INPUT

        position += n
        bytesRemaining -= n
        bytesTransferred(n)
        return n
    }

    private fun readPlaintext(pos: Long, dst: ByteArray, dstOffset: Int, length: Int): Int {
        reader?.let { return it.readInto(pos, dst, dstOffset, length) }
        val plain = v1Plain ?: return -1
        if (pos >= plain.size) return -1
        val n = minOf(length.toLong(), plain.size - pos).toInt()
        System.arraycopy(plain, pos.toInt(), dst, dstOffset, n)
        return n
    }

    override fun getUri(): Uri? = if (usingDefault) defaultSource.uri else uri

    override fun close() {
        val wasDefault = usingDefault
        try {
            if (wasDefault) defaultSource.close()
        } finally {
            uri = null
            position = 0
            bytesRemaining = 0
            usingDefault = false
            // Keep resolved geometry cached across close→open so seeks are cheap;
            // it is dropped when a different blob opens or the instance is GC'd.
            if (!wasDefault) transferEnded()
        }
    }

    @UnstableApi
    class Factory(
        private val context: Context,
        private val blobStream: EncryptedBlobStream,
    ) : DataSource.Factory {
        override fun createDataSource(): DataSource = MediaBlobDataSource(context, blobStream)
    }
}
