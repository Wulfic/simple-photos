/**
 * AES-256-GCM encryption/decryption operations and SHA-256 hashing utilities
 * for the Simple Photos Android client.
 */
package com.simplephotos.crypto

import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec

/**
 * The minimal AES-GCM surface that format-aware blob helpers (e.g. [ChunkedBlob])
 * depend on. [CryptoManager] is the production implementation; extracting it as an
 * interface lets the chunked encode/decode framing be unit-tested on the JVM with
 * an in-memory key, without the Android Keystore that [KeyManager] requires.
 */
interface Cryptor {
    /** Encrypt [plaintext], returning `nonce || ciphertext+tag`. */
    fun encrypt(plaintext: ByteArray): ByteArray

    /** Decrypt a `nonce || ciphertext+tag` payload produced by [encrypt]. */
    fun decrypt(data: ByteArray): ByteArray
}

/**
 * Provides AES-256-GCM encryption and decryption using the Data Encryption Key
 * managed by [KeyManager].
 *
 * Every [encrypt] call generates a fresh 12-byte random nonce. The output format
 * is `nonce || ciphertext+tag`, matching the [EncryptedPayload] wire format used
 * by the Android client, the web client, and the server.
 */
class CryptoManager(private val keyManager: KeyManager) : Cryptor {

    companion object {
        private const val NONCE_SIZE = 12
        /** GCM authentication tag length in bits. */
        private const val TAG_SIZE = 128
    }

    /** Encrypt [plaintext] with a fresh random nonce. Returns `nonce || ciphertext+tag`. */
    override fun encrypt(plaintext: ByteArray): ByteArray {
        val key = keyManager.loadKey() ?: throw IllegalStateException("Encryption key not available")
        val nonce = ByteArray(NONCE_SIZE)
        SecureRandom().nextBytes(nonce)

        // Fresh nonce per encryption; reviewed.
        val cipher = Cipher.getInstance("AES/GCM/NoPadding") // nosemgrep: kotlin.lang.security.gcm-detection.gcm-detection
        cipher.init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(TAG_SIZE, nonce)) // nosemgrep: kotlin.lang.security.gcm-detection.gcm-detection
        val ciphertext = cipher.doFinal(plaintext)

        return EncryptedPayload(nonce, ciphertext).toByteArray()
    }

    override fun decrypt(data: ByteArray): ByteArray {
        val key = keyManager.loadKey() ?: throw IllegalStateException("Encryption key not available")
        val payload = EncryptedPayload.fromByteArray(data)

        // Nonce read from sealed payload; reviewed.
        val cipher = Cipher.getInstance("AES/GCM/NoPadding") // nosemgrep: kotlin.lang.security.gcm-detection.gcm-detection
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_SIZE, payload.nonce)) // nosemgrep: kotlin.lang.security.gcm-detection.gcm-detection
        return cipher.doFinal(payload.ciphertext)
    }

    fun sha256Hex(data: ByteArray): String {
        val digest = MessageDigest.getInstance("SHA-256")
        return digest.digest(data).joinToString("") { "%02x".format(it) }
    }
}
