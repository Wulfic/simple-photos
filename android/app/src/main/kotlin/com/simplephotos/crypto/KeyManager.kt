package com.simplephotos.crypto

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import org.bouncycastle.crypto.generators.Argon2BytesGenerator
import org.bouncycastle.crypto.params.Argon2Parameters
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Manages the AES-256-GCM Data Encryption Key (DEK) derived from the user's
 * login password via Argon2id.
 *
 * The salt is deterministic — SHA-256("simple-photos:" + username).slice(0, 16)
 * — so the same username + password always produces the same key on both
 * Android and the web client. The DEK is wrapped with an Android Keystore key
 * and stored in EncryptedSharedPreferences so the user doesn't need to re-enter
 * the password on every app launch.
 */
class KeyManager(private val context: Context) {

    companion object {
        private const val KEYSTORE_ALIAS = "simple_photos_dek_wrapper"
        private const val PREFS_NAME = "simple_photos_keys"
        private const val KEY_WRAPPED_DEK = "wrapped_dek"
        private const val KEY_WRAPPED_DEK_IV = "wrapped_dek_iv"
        private const val ARGON2_MEMORY = 65536  // 64 MB
        private const val ARGON2_ITERATIONS = 3
        private const val ARGON2_PARALLELISM = 4
        private const val DEK_SIZE = 32         // AES-256
    }

    private var dek: SecretKey? = null

    private val prefs by lazy {
        val masterKey = MasterKey.Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()
        EncryptedSharedPreferences.create(
            context, PREFS_NAME, masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
        )
    }

    val isKeyConfigured: Boolean
        get() = prefs.contains(KEY_WRAPPED_DEK)

    /**
     * Derive the encryption key from the user's login password + username.
     *
     * Salt = SHA-256("simple-photos:" + username).slice(0, 16)
     * This matches the web client's derivation exactly.
     */
    fun deriveAndStoreKey(password: String, username: String) {
        // Deterministic 16-byte salt — matches web: SHA-256("simple-photos:" + username).slice(0, 16)
        val saltInput = "simple-photos:$username".toByteArray(Charsets.UTF_8)
        val fullHash = MessageDigest.getInstance("SHA-256").digest(saltInput)
        val salt = fullHash.copyOf(16)

        // Argon2id key derivation
        val dekBytes = ByteArray(DEK_SIZE)
        val params = Argon2Parameters.Builder(Argon2Parameters.ARGON2_id)
            .withSalt(salt)
            .withMemoryAsKB(ARGON2_MEMORY)
            .withIterations(ARGON2_ITERATIONS)
            .withParallelism(ARGON2_PARALLELISM)
            .build()
        val generator = Argon2BytesGenerator()
        generator.init(params)
        generator.generateBytes(password.toByteArray(Charsets.UTF_8), dekBytes)

        val derivedKey = SecretKeySpec(dekBytes, "AES")

        // Wrap with Android Keystore for at-rest protection
        val wrapperKey = getOrCreateWrapperKey()
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, wrapperKey)
        val wrappedDek = cipher.doFinal(dekBytes)
        val iv = cipher.iv

        // Persist wrapped DEK
        prefs.edit()
            .putString(KEY_WRAPPED_DEK, wrappedDek.toHex())
            .putString(KEY_WRAPPED_DEK_IV, iv.toHex())
            .apply()

        dek = derivedKey
        dekBytes.fill(0)
    }

    fun loadKey(): SecretKey? {
        if (dek != null) return dek

        val wrappedHex = prefs.getString(KEY_WRAPPED_DEK, null) ?: return null
        val ivHex = prefs.getString(KEY_WRAPPED_DEK_IV, null) ?: return null

        val wrapperKey = getOrCreateWrapperKey()
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, wrapperKey, GCMParameterSpec(128, ivHex.hexToBytes()))
        val dekBytes = cipher.doFinal(wrappedHex.hexToBytes())

        dek = SecretKeySpec(dekBytes, "AES")
        dekBytes.fill(0)
        return dek
    }

    /**
     * Return the raw DEK as a lowercase hex string (64 chars for AES-256).
     * Returns null if the key has not been derived / loaded yet.
     */
    fun getKeyHex(): String? {
        val key = loadKey() ?: return null
        return key.encoded.joinToString("") { "%02x".format(it) }
    }

    fun clearKey() {
        dek = null
        prefs.edit()
            .remove(KEY_WRAPPED_DEK)
            .remove(KEY_WRAPPED_DEK_IV)
            .apply()
    }

    private fun getOrCreateWrapperKey(): SecretKey {
        val keyStore = KeyStore.getInstance("AndroidKeyStore")
        keyStore.load(null)

        if (keyStore.containsAlias(KEYSTORE_ALIAS)) {
            return (keyStore.getEntry(KEYSTORE_ALIAS, null) as KeyStore.SecretKeyEntry).secretKey
        }

        val spec = KeyGenParameterSpec.Builder(
            KEYSTORE_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .build()

        val keyGenerator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        keyGenerator.init(spec)
        return keyGenerator.generateKey()
    }

    private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
    private fun String.hexToBytes(): ByteArray {
        val len = length
        val data = ByteArray(len / 2)
        for (i in 0 until len step 2) {
            data[i / 2] = ((Character.digit(this[i], 16) shl 4) + Character.digit(this[i + 1], 16)).toByte()
        }
        return data
    }
}
