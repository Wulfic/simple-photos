package com.simplephotos.ui.navigation

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import javax.inject.Inject

@HiltViewModel
class NavViewModel @Inject constructor(
    private val dataStore: DataStore<Preferences>
) : ViewModel() {

    companion object {
        val KEY_SERVER_CONFIGURED = booleanPreferencesKey("server_configured")
        val KEY_ACCESS_TOKEN = stringPreferencesKey("access_token")
        val KEY_REFRESH_TOKEN = stringPreferencesKey("refresh_token")
        val KEY_SERVER_URL = stringPreferencesKey("server_url")
        val KEY_USERNAME = stringPreferencesKey("username")
        val KEY_DIAGNOSTIC_LOGGING = booleanPreferencesKey("diagnostic_logging")
        val KEY_BIOMETRIC_ENABLED = booleanPreferencesKey("biometric_enabled")
    }

    private val _startDestination = MutableStateFlow<String?>(null)
    val startDestination: StateFlow<String?> = _startDestination

    init {
        viewModelScope.launch {
            val prefs = dataStore.data.first()
            val serverConfigured = prefs[KEY_SERVER_CONFIGURED] ?: false
            val hasToken = prefs[KEY_ACCESS_TOKEN] != null

            _startDestination.value = when {
                !serverConfigured -> Screen.ServerSetup.route
                !hasToken -> Screen.Login.route
                else -> Screen.Gallery.route
            }
        }
    }
}
