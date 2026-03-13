package com.simplephotos.data.repository

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.work.WorkManager
import coil.ImageLoader
import coil.imageLoader
import com.simplephotos.crypto.KeyManager
import com.simplephotos.data.local.AppDatabase
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import com.simplephotos.sync.SyncScheduler
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_ACCESS_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_REFRESH_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Handles authentication (register, login, TOTP, token refresh) and full
 * logout with cleanup of all local caches so no stale data leaks between
 * user sessions.
 *
 * On login, derives the E2E encryption key via [KeyManager.deriveAndStoreKey]
 * using the same deterministic salt as the web client, ensuring cross-platform
 * decryption compatibility.
 */
@Singleton
class AuthRepository @Inject constructor(
    private val api: ApiService,
    private val dataStore: DataStore<Preferences>,
    private val keyManager: KeyManager,
    private val db: AppDatabase,
    @ApplicationContext private val context: Context
) {
    suspend fun register(username: String, password: String): RegisterResponse =
        api.register(RegisterRequest(username, password))

    /**
     * Login and derive encryption key from password + username.
     * The key derivation uses the same deterministic salt as the web client,
     * so photos encrypted on either platform can be decrypted on the other.
     */
    suspend fun login(username: String, password: String): LoginResponse {
        val response = api.login(LoginRequest(username, password))
        if (response.accessToken != null && response.refreshToken != null) {
            saveTokens(response.accessToken, response.refreshToken, username)
            // Derive and store encryption key from password (matches web crypto)
            withContext(Dispatchers.IO) {
                keyManager.deriveAndStoreKey(password, username)
            }
        }
        return response
    }

    suspend fun loginTotp(
        sessionToken: String,
        totpCode: String?,
        backupCode: String?,
        password: String,
        username: String
    ): LoginResponse {
        val response = api.loginTotp(TotpLoginRequest(sessionToken, totpCode, backupCode))
        if (response.accessToken != null && response.refreshToken != null) {
            saveTokens(response.accessToken, response.refreshToken, username)
            // Derive key after TOTP verification completes the login
            withContext(Dispatchers.IO) {
                keyManager.deriveAndStoreKey(password, username)
            }
        }
        return response
    }

    suspend fun refreshToken(): String? {
        val prefs = dataStore.data.first()
        val refreshToken = prefs[KEY_REFRESH_TOKEN] ?: return null
        return try {
            val response = api.refresh(RefreshRequest(refreshToken))
            dataStore.edit { it[KEY_ACCESS_TOKEN] = response.accessToken }
            response.accessToken
        } catch (_: Exception) {
            null
        }
    }

    /**
     * Full logout: clear server session, all local caches, and scheduled work.
     *
     * Prevents stale data from flashing when another user logs in:
     *  - Room DB (photos, albums, cross-refs, blob queue, backup folders)
     *  - Thumbnail files on disk
     *  - Coil in-memory image cache
     *  - WorkManager backup workers
     *  - DataStore preferences (tokens, settings)
     *  - Encryption key
     */
    suspend fun logout() {
        // 1. Notify server (best-effort)
        val prefs = dataStore.data.first()
        prefs[KEY_REFRESH_TOKEN]?.let { token ->
            try { api.logout(LogoutRequest(token)) } catch (_: Exception) {}
        }

        // 2. Cancel all scheduled backup workers
        SyncScheduler.cancel(context)

        // 3. Clear Room database tables
        db.photoDao().deleteAll()
        db.albumDao().deleteAll()
        db.albumDao().deleteAllXRefs()
        db.blobQueueDao().deleteAll()
        db.backupFolderDao().deleteAll()

        // 4. Delete cached thumbnail files from disk
        val thumbnailDir = File(context.filesDir, "thumbnails")
        if (thumbnailDir.exists()) {
            thumbnailDir.deleteRecursively()
        }

        // 5. Clear Coil in-memory bitmap cache
        context.imageLoader.memoryCache?.clear()

        // 6. Clear auth tokens and encryption key
        dataStore.edit { it.clear() }
        keyManager.clearKey()
    }

    suspend fun setup2fa(): TotpSetupResponse = api.setup2fa()

    suspend fun confirm2fa(code: String) {
        api.confirm2fa(TotpConfirmRequest(code))
    }

    suspend fun disable2fa(code: String) {
        api.disable2fa(TotpDisableRequest(code))
    }

    private suspend fun saveTokens(access: String, refresh: String, username: String?) {
        dataStore.edit { prefs ->
            prefs[KEY_ACCESS_TOKEN] = access
            prefs[KEY_REFRESH_TOKEN] = refresh
            if (username != null) prefs[KEY_USERNAME] = username
        }
    }
}
