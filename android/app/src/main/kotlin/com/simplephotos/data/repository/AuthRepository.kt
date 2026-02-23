package com.simplephotos.data.repository

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import com.simplephotos.crypto.KeyManager
import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_ACCESS_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_REFRESH_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class AuthRepository @Inject constructor(
    private val api: ApiService,
    private val dataStore: DataStore<Preferences>,
    private val keyManager: KeyManager
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

    suspend fun logout() {
        val prefs = dataStore.data.first()
        prefs[KEY_REFRESH_TOKEN]?.let { token ->
            try { api.logout(LogoutRequest(token)) } catch (_: Exception) {}
        }
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
