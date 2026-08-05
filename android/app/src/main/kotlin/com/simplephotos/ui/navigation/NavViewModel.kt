/**
 * ViewModel that resolves the start destination and decodes admin status
 * from the persisted JWT access token.
 */
package com.simplephotos.ui.navigation

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import android.util.Base64
import com.simplephotos.data.repository.AuthRepository
import org.json.JSONObject
import javax.inject.Inject

/**
 * Determines the start destination (server setup → login → gallery) based on
 * persisted DataStore preferences, and decodes the admin role from the JWT
 * access token to gate admin-only UI features.
 */
@HiltViewModel
class NavViewModel @Inject constructor(
    // Public so screens that render the shared AppHeader navbar (#35) can drive
    // the theme toggle through it, matching AlbumViewModel.dataStore.
    val dataStore: DataStore<Preferences>,
    private val authRepository: AuthRepository,
) : ViewModel() {

    companion object {
        val KEY_SERVER_CONFIGURED = booleanPreferencesKey("server_configured")
        val KEY_ACCESS_TOKEN = stringPreferencesKey("access_token")
        val KEY_REFRESH_TOKEN = stringPreferencesKey("refresh_token")
        val KEY_SERVER_URL = stringPreferencesKey("server_url")
        val KEY_USERNAME = stringPreferencesKey("username")
        val KEY_DIAGNOSTIC_LOGGING = booleanPreferencesKey("diagnostic_logging")
        val KEY_BIOMETRIC_ENABLED = booleanPreferencesKey("biometric_enabled")
        val KEY_THUMBNAIL_SIZE = stringPreferencesKey("thumbnail_size")
        val KEY_WIFI_ONLY_BACKUP = booleanPreferencesKey("wifi_only_backup")
    }

    private val _startDestination = MutableStateFlow<String?>(null)
    val startDestination: StateFlow<String?> = _startDestination

    private val _isAdmin = MutableStateFlow(false)
    val isAdmin: StateFlow<Boolean> = _isAdmin

    // Logged-in username for the shared AppHeader avatar/dropdown (#35).
    private val _username = MutableStateFlow("")
    val username: StateFlow<String> = _username

    init {
        viewModelScope.launch {
            val prefs = dataStore.data.first()
            val serverConfigured = prefs[KEY_SERVER_CONFIGURED] ?: false
            val hasToken = prefs[KEY_ACCESS_TOKEN] != null

            // Decode role from JWT access token
            val token = prefs[KEY_ACCESS_TOKEN]
            _isAdmin.value = decodeAdminFromJwt(token)
            _username.value = prefs[KEY_USERNAME] ?: ""

            _startDestination.value = when {
                !serverConfigured -> Screen.ServerSetup.route
                !hasToken -> Screen.Login.route
                else -> Screen.Gallery.route
            }
        }
    }

    /** Sign out (clears tokens server- and client-side), then invoke [onLoggedOut]
     *  to navigate. Mirrors AlbumViewModel.logout so the navbar Sign Out behaves
     *  identically from every screen (#35). */
    fun logout(onLoggedOut: () -> Unit) {
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { authRepository.logout() }
            } catch (_: Exception) {}
            onLoggedOut()
        }
    }

    /** Decode the "role" claim from a JWT access token. Returns true if role == "admin". */
    private fun decodeAdminFromJwt(token: String?): Boolean {
        if (token == null) return false
        return try {
            val payload = token.split(".").getOrNull(1) ?: return false
            val decoded = String(Base64.decode(payload, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP))
            val json = JSONObject(decoded)
            json.optString("role") == "admin"
        } catch (_: Exception) {
            false
        }
    }
}
