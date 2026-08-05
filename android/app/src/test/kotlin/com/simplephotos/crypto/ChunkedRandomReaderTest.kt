package com.simplephotos.crypto

import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Unit tests for [ChunkedRandomReader] and the v2 geometry helpers
 * ([ChunkedBlob.plaintextTotalOf], [ChunkedBlob.chunksStart]) — the seek/slice
 * arithmetic behind streaming encrypted video (issue #17).
 *
 * The container under test is produced by the *real* encoder
 * ([ChunkedBlob.encryptStreamToFile]) with a real AES-GCM [Cryptor], so a green
 * test proves the reader's frame math against production framing byte-for-byte
 * without any device. [fetchBlock] slices the in-memory container exactly as an
 * HTTP Range request would (clamped at EOF).
 */
class ChunkedRandomReaderTest {

    /** In-memory AES-256-GCM, `nonce(12) || ciphertext+tag`, mirroring CryptoManager. */
    private class FakeCryptor : Cryptor {
        private val key = SecretKeySpec(ByteArray(32) { (it * 7 + 1).toByte() }, "AES")
        private val rng = SecureRandom()
        override fun encrypt(plaintext: ByteArray): ByteArray {
            val nonce = ByteArray(12).also { rng.nextBytes(it) }
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(128, nonce))
            return nonce + cipher.doFinal(plaintext)
        }
        override fun decrypt(data: ByteArray): ByteArray {
            val nonce = data.copyOf(12)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, nonce))
            return cipher.doFinal(data.copyOfRange(12, data.size))
        }
    }

    private fun payload(size: Int): ByteArray = ByteArray(size) { ((it * 31 + 7) and 0xFF).toByte() }

    /** Build a real v2 container for [source] and a reader over it. */
    private fun readerFor(source: ByteArray): Pair<ChunkedRandomReader, ByteArray> {
        val crypto = FakeCryptor()
        val dst = java.io.File.createTempFile("crr_", ".spchnk").also { it.deleteOnExit() }
        ChunkedBlob.encryptStreamToFile(crypto, source.inputStream(), dst, "{}".toByteArray())
        val container = dst.readBytes()

        val metaLen = ChunkedBlob.readU32BE(container, ChunkedBlob.MAGIC_SIZE)
        val chunksStart = ChunkedBlob.chunksStart(metaLen)
        val plaintextTotal = ChunkedBlob.plaintextTotalOf(container.size.toLong(), chunksStart)

        val fetchBlock: (Long, Long) -> ByteArray = { off, len ->
            val start = off.toInt()
            val end = minOf(container.size.toLong(), off + len).toInt()
            container.copyOfRange(start, end)
        }
        val reader = ChunkedRandomReader(chunksStart, plaintextTotal, fetchBlock, crypto::decrypt)
        return reader to container
    }

    /** Read the whole stream through the reader, one [readInto] call at a time. */
    private fun readAll(reader: ChunkedRandomReader, chunk: Int = 65_536): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        val buf = ByteArray(chunk)
        var pos = 0L
        while (true) {
            val n = reader.readInto(pos, buf, 0, buf.size)
            if (n < 0) break
            out.write(buf, 0, n)
            pos += n
        }
        return out.toByteArray()
    }

    @Test
    fun plaintextTotalMatchesSourceAcrossFrameBoundaries() {
        for (size in intArrayOf(
            0,                                   // empty
            4096,                                // sub-chunk (single short frame)
            ChunkedBlob.CHUNK_SIZE,              // exactly one full frame
            ChunkedBlob.CHUNK_SIZE * 3,          // exact multiple (no tail)
            ChunkedBlob.CHUNK_SIZE * 2 + 123_456 // full frames + partial tail
        )) {
            val (reader, _) = readerFor(payload(size))
            assertEquals("plaintextTotal for size=$size", size.toLong(), reader.plaintextTotal)
        }
    }

    @Test
    fun sequentialReadRecoversExactSource() {
        val source = payload(ChunkedBlob.CHUNK_SIZE * 2 + 123_456)
        val (reader, _) = readerFor(source)
        assertArrayEquals(source, readAll(reader))
    }

    @Test
    fun seekIntoLaterFrameReturnsCorrectBytes() {
        val source = payload(ChunkedBlob.CHUNK_SIZE * 3 + 5000)
        val (reader, _) = readerFor(source)

        // Seek to a position deep in the 3rd frame and read across into the 4th,
        // exactly what ExoPlayer does when it seeks to a keyframe.
        val start = ChunkedBlob.CHUNK_SIZE * 2L + 1000
        val want = ChunkedBlob.CHUNK_SIZE + 2000 // spans the frame boundary
        val got = java.io.ByteArrayOutputStream()
        val buf = ByteArray(40_000)
        var pos = start
        while (pos < start + want) {
            val n = reader.readInto(pos, buf, 0, minOf(buf.size.toLong(), start + want - pos).toInt())
            if (n < 0) break
            got.write(buf, 0, n)
            pos += n
        }
        assertArrayEquals(
            source.copyOfRange(start.toInt(), (start + want).toInt()),
            got.toByteArray(),
        )
    }

    @Test
    fun readAtOrPastEndReturnsMinusOne() {
        val source = payload(ChunkedBlob.CHUNK_SIZE + 10)
        val (reader, _) = readerFor(source)
        val buf = ByteArray(16)
        assertEquals(-1, reader.readInto(reader.plaintextTotal, buf, 0, buf.size))
        assertEquals(-1, reader.readInto(reader.plaintextTotal + 100, buf, 0, buf.size))
    }

    @Test
    fun readHonoursDestinationOffsetAndLength() {
        val source = payload(ChunkedBlob.CHUNK_SIZE + 500)
        val (reader, _) = readerFor(source)
        val buf = ByteArray(100)
        // Ask for 10 bytes at plaintext offset 42, written into buf at offset 7.
        val n = reader.readInto(42, buf, 7, 10)
        assertEquals(10, n)
        assertArrayEquals(source.copyOfRange(42, 52), buf.copyOfRange(7, 17))
    }
}
