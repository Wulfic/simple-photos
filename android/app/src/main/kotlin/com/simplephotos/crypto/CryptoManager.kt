package com.simplephotos.crypto

import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec

/**
 * Provides AES-256-GCM encryption and decryption using the Data Encryption Key
 * managed by [KeyManager].
 *
 * Every [encrypt] call generates a fresh 12-byte random nonce. The output format
 * is `nonce || ciphertext+tag`, matching the [EncryptedPayload] wire format used
 * by the Android client, the web client, and the server.
 */
class CryptoManager(private val keyManager: KeyManager) {

    companion object {
        private const val NONCE_SIZE = 12
        /** GCM authentication tag length in bits. */
        private const val TAG_SIZE = 128
    }

    /** Encrypt [plaintext] with a fresh random nonce. Returns `nonce || ciphertext+tag`. */
    fun encrypt(plaintext: ByteArray): ByteArray {
        val key = keyManager.loadKey() ?: throw IllegalStateException("Encryption key not available")
        val nonce = ByteArray(NONCE_SIZE)
        SecureRandom().nextBytes(nonce)

        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(TAG_SIZE, nonce))
        val ciphertext = cipher.doFinal(plaintext)

        return EncryptedPayload(nonce, ciphertext).toByteArray()
    }

    fun decrypt(data: ByteArray): ByteArray {
        val key = keyManager.loadKey() ?: throw IllegalStateException("Encryption key not available")
        val payload = EncryptedPayload.fromByteArray(data)

        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_SIZE, payload.nonce))
        return cipher.doFinal(payload.ciphertext)
    }

    fun sha256Hex(data: ByteArray): String {
        val digest = MessageDigest.getInstance("SHA-256")
        return digest.digest(data).joinToString("") { "%02x".format(it) }
    }
}
