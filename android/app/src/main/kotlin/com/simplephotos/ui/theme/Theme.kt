package com.simplephotos.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

private val LightColorScheme = lightColorScheme()
private val DarkColorScheme = darkColorScheme()

/** Persisted theme preference: "light", "dark", or "system" */
val KEY_THEME_MODE = stringPreferencesKey("theme_mode")

/** Global observable theme state, shared across all screens. */
object ThemeState {
    var mode by mutableStateOf("system")
        private set

    fun init(dataStore: DataStore<Preferences>) {
        mode = runBlocking {
            dataStore.data.first()[KEY_THEME_MODE] ?: "system"
        }
    }

    fun toggle(dataStore: DataStore<Preferences>, isCurrentlyDark: Boolean? = null) {
        // When in "system" mode, the simple mode == "dark" check is wrong
        // because mode is "system" even though the visual theme may be dark.
        // Accept the resolved dark state so the first toggle always flips visually.
        val effectivelyDark = isCurrentlyDark ?: (mode == "dark")
        mode = if (effectivelyDark) "light" else "dark"
        CoroutineScope(Dispatchers.IO).launch {
            dataStore.edit { it[KEY_THEME_MODE] = mode }
        }
    }

    fun isDark(systemDark: Boolean): Boolean = when (mode) {
        "dark" -> true
        "light" -> false
        else -> systemDark
    }
}

@Composable
fun SimplePhotosTheme(
    darkTheme: Boolean = ThemeState.isDark(isSystemInDarkTheme()),
    content: @Composable () -> Unit
) {
    val colorScheme = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        darkTheme -> DarkColorScheme
        else -> LightColorScheme
    }

    MaterialTheme(
        colorScheme = colorScheme,
        content = content
    )
}
