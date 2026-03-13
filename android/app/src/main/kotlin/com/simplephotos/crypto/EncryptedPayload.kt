package com.simplephotos.crypto

/**
 * Wire format for AES-256-GCM encrypted data: a 12-byte nonce followed by the
 * ciphertext (which includes the GCM authentication tag appended by the JCE).
 *
 * Layout: `[nonce (12 bytes)][ciphertext + GCM tag]`
 *
 * Custom [equals] and [hashCode] use content-based comparison since [ByteArray]
 * defaults to reference equality.
 */
data class EncryptedPayload(
    val nonce: ByteArray,
    val ciphertext: ByteArray
) {
    fun toByteArray(): ByteArray {
        val result = ByteArray(nonce.size + ciphertext.size)
        System.arraycopy(nonce, 0, result, 0, nonce.size)
        System.arraycopy(ciphertext, 0, result, nonce.size, ciphertext.size)
        return result
    }

    companion object {
        private const val NONCE_SIZE = 12

        fun fromByteArray(data: ByteArray): EncryptedPayload {
            require(data.size > NONCE_SIZE) { "Data too short to contain nonce + ciphertext" }
            val nonce = data.copyOfRange(0, NONCE_SIZE)
            val ciphertext = data.copyOfRange(NONCE_SIZE, data.size)
            return EncryptedPayload(nonce, ciphertext)
        }
    }

    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is EncryptedPayload) return false
        return nonce.contentEquals(other.nonce) && ciphertext.contentEquals(other.ciphertext)
    }

    override fun hashCode(): Int = 31 * nonce.contentHashCode() + ciphertext.contentHashCode()
}
