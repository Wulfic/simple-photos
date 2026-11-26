/**
 * Repository for authentication, session management, and credential storage.
 */
package com.simplephotos.data.repository

import android.content.Context
import android.util.Log
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
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_REFRESH_TOKEN
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Handles authentication (register, login, TOTP), session expiry, and full
 * logout with cleanup of all local caches so no stale data leaks between
 * user sessions. Token refresh lives in [com.simplephotos.data.remote.SessionRefresher].
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
    private val secureGalleryRepository: SecureGalleryRepository,
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
            establishSession(response.accessToken, response.refreshToken, username, password)
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
            establishSession(response.accessToken, response.refreshToken, username, password)
        }
        return response
    }

    /**
     * The server issued tokens for [username]. An expired session (N1c) leaves
     * the previous account's mirror, key and backup folders on the device so a
     * re-login re-encrypts nothing — which is only safe if it IS that account.
     * Anyone else (or an unknown owner) gets today's full local wipe first.
     */
    private suspend fun establishSession(access: String, refresh: String, username: String, password: String) {
        if (SessionPrefs.mustWipeBeforeLogin(dataStore.data.first(), username)) {
            val photos = db.photoDao().countAll()
            val folders = db.backupFolderDao().count()
            Log.i(TAG, "Login: local data is not this account's — wiping $photos photos, $folders backup folders first")
            wipeLocalAccount()
        } else {
            Log.i(TAG, "Login: same account as the local mirror — keeping it")
        }
        dataStore.edit { SessionPrefs.recordLogin(it, access, refresh, username) }
        // Derive and store encryption key from password (matches web crypto)
        withContext(Dispatchers.IO) {
            keyManager.deriveAndStoreKey(password, username)
        }
    }

    /**
     * The server refused [deadRefreshToken] (N1c). Drop the session's tokens —
     * nothing else; see [SessionPrefs.expire].
     *
     * @return true when the device now has no live session (route to Login).
     */
    suspend fun expireSession(deadRefreshToken: String): Boolean {
        var noSession = false
        dataStore.edit { noSession = SessionPrefs.expire(it, deadRefreshToken) }
        if (noSession) {
            // A re-login must re-unlock secure albums, as after a logout.
            com.simplephotos.data.remote.GalleryTokenHolder.token = null
            Log.w(TAG, "Session expired (refresh token refused); library kept, login required")
        }
        return noSession
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
        // 1. Notify server (best-effort: the local wipe must happen regardless)
        val prefs = dataStore.data.first()
        prefs[KEY_REFRESH_TOKEN]?.let { token ->
            try {
                api.logout(LogoutRequest(token))
            } catch (e: Exception) {
                Log.w(TAG, "Server logout failed (${e.javaClass.simpleName}); wiping locally anyway")
            }
        }
        wipeLocalAccount()
    }

    /**
     * Everything [logout] removes locally. Also run before a DIFFERENT account
     * logs in over an expired session's kept mirror ([establishSession]).
     * Keeps only which server the device is set up for.
     */
    private suspend fun wipeLocalAccount() {
        // 2. Cancel all scheduled backup workers
        SyncScheduler.cancel(context)

        // 3. Clear Room database tables
        db.photoDao().deleteAll()
        db.albumDao().deleteAll()
        db.albumDao().deleteAllXRefs()
        db.blobQueueDao().deleteAll()
        db.backupFolderDao().deleteAll()
        // The #38 delta cursor MUST go with the mirror. Living in the same Room
        // database is not enough on its own, because this method clears tables
        // one DAO at a time rather than dropping the file — leave it behind and
        // the next user's first sync starts from a cursor describing the
        // previous user's library, skips straight to a delta, and shows an
        // empty gallery that no amount of re-syncing repairs.
        db.syncStateDao().deleteAll()

        // 4. Delete cached thumbnail files from disk
        val thumbnailDir = File(context.filesDir, "thumbnails")
        if (thumbnailDir.exists()) {
            thumbnailDir.deleteRecursively()
        }

        // 5. Clear Coil in-memory bitmap cache
        context.imageLoader.memoryCache?.clear()

        // 6. Clear auth tokens and encryption key — but not the server: "log out"
        // used to mean "forget the server" too (N1c). This also drops the persisted
        // secure-blob-id fallback (B5), but that repository is a @Singleton whose
        // in-memory mirror outlives the account within one process, so it has to
        // be told explicitly — same reasoning as the sync cursor above.
        secureGalleryRepository.forgetSecureBlobIds()
        dataStore.edit { SessionPrefs.clearForLogout(it) }
        keyManager.clearKey()
        // Drop the secure-album unlock token so a re-login must re-unlock.
        com.simplephotos.data.remote.GalleryTokenHolder.token = null
    }

    suspend fun setup2fa(): TotpSetupResponse = api.setup2fa()

    suspend fun confirm2fa(code: String) {
        api.confirm2fa(TotpConfirmRequest(code))
    }

    suspend fun disable2fa(code: String) {
        api.disable2fa(TotpDisableRequest(code))
    }

    private companion object {
        const val TAG = "AuthRepository"
    }
}
