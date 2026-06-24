package com.simplephotos.crypto

import java.io.File
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for [ChunkedBlob]'s v2 streaming producer ([ChunkedBlob.encryptStreamToFile])
 * and its decrypt counterparts. The framing must match the server's `blobs/chunked.rs`
 * byte-for-byte, so these exercise encode → decode round-trips and the digest contract
 * the uploader relies on. A pure-JVM [Cryptor] stands in for [CryptoManager] (which needs
 * the Android Keystore) — encrypt and decrypt are paired, so any AES-GCM impl proves the
 * framing as long as it's self-consistent.
 */
class ChunkedBlobTest {

    /** In-memory AES-256-GCM, `nonce(12) || ciphertext+tag`, mirroring [CryptoManager]. */
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

    private fun tmp(suffix: String): File =
        File.createTempFile("chunkedblob_test_", suffix).also { it.deleteOnExit() }

    private fun sha256Hex(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    /** Deterministic pseudo-random payload of [size] bytes. */
    private fun payload(size: Int): ByteArray = ByteArray(size) { ((it * 31 + 7) and 0xFF).toByte() }

    @Test
    fun encryptThenStreamDecryptRoundTripsMultipleChunks() {
        val crypto = FakeCryptor()
        // > 2× CHUNK_SIZE so we exercise two full frames plus a partial tail.
        val source = payload(ChunkedBlob.CHUNK_SIZE * 2 + 123_456)
        val meta = """{"v":2,"filename":"big.mp4","chunk_size":${ChunkedBlob.CHUNK_SIZE}}""".toByteArray()

        val dst = tmp(".spchnk")
        val result = ChunkedBlob.encryptStreamToFile(crypto, source.inputStream(), dst, meta)

        // Container is recognisably v2 and self-reports its true on-disk size.
        val written = dst.readBytes()
        assertTrue("written container must carry the v2 magic", ChunkedBlob.isChunked(written))
        assertEquals(written.size.toLong(), result.blobSize)
        // Streaming must not inflate like v1 base64 did — container ≈ source + framing overhead.
        assertTrue("v2 container should be near 1× source, not ~1.37×", result.blobSize < source.size + 1_000_000L)

        // Digest contract: content hash = sha256(source)[..6], client hash = sha256(container).
        assertEquals(sha256Hex(source).take(12), result.contentHashHex)
        assertEquals(sha256Hex(written), result.clientHashHex)

        // Stream-decrypt path recovers the exact source bytes.
        val out = tmp(".bin")
        ChunkedBlob.decryptChunkedStreamToFile(crypto, written.inputStream(), out)
        assertArrayEquals(source, out.readBytes())
    }

    @Test
    fun encryptThenInMemoryDecryptRoundTrips() {
        val crypto = FakeCryptor()
        val source = payload(CHUNK_AND_A_BIT)
        val dst = tmp(".spchnk")
        ChunkedBlob.encryptStreamToFile(crypto, source.inputStream(), dst, "{}".toByteArray())

        // The in-memory path (used for Coil-rendered photos) must agree with the
        // streaming path on the same container.
        val recovered = ChunkedBlob.decryptPhotoBlobBytes(crypto, dst.readBytes())
        assertArrayEquals(source, recovered)
    }

    @Test
    fun emptySourceProducesValidContainerThatDecryptsToNothing() {
        val crypto = FakeCryptor()
        val dst = tmp(".spchnk")
        val result = ChunkedBlob.encryptStreamToFile(crypto, ByteArray(0).inputStream(), dst, "{}".toByteArray())

        assertTrue(ChunkedBlob.isChunked(dst.readBytes()))
        assertEquals(sha256Hex(ByteArray(0)).take(12), result.contentHashHex)

        val out = tmp(".bin")
        ChunkedBlob.decryptChunkedStreamToFile(crypto, dst.readBytes().inputStream(), out)
        assertEquals(0L, out.length())
    }

    @Test
    fun contentHashTracksSourceNotEncryptedBytes() {
        val crypto = FakeCryptor()
        val source = payload(CHUNK_AND_A_BIT)

        // Same source twice → different containers (fresh nonces) but identical
        // content hash, so server-side dedup still recognises the original.
        val a = tmp(".spchnk").let { ChunkedBlob.encryptStreamToFile(crypto, source.inputStream(), it, "{}".toByteArray()) }
        val b = tmp(".spchnk").let { ChunkedBlob.encryptStreamToFile(crypto, source.inputStream(), it, "{}".toByteArray()) }

        assertEquals(a.contentHashHex, b.contentHashHex)
        assertNotEquals("fresh nonces must yield distinct containers", a.clientHashHex, b.clientHashHex)
    }

    private companion object {
        /** A size that spans one full chunk plus a partial tail. */
        const val CHUNK_AND_A_BIT = ChunkedBlob.CHUNK_SIZE + 4096
    }
}
