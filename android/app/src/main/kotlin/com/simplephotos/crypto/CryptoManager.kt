package com.simplephotos.crypto

import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec

class CryptoManager(private val keyManager: KeyManager) {

    companion object {
        private const val NONCE_SIZE = 12
        private const val TAG_SIZE = 128
    }

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
