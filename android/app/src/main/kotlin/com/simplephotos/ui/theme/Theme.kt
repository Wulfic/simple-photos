/**
 * Material 3 theme configuration with persisted light/dark/system mode.
 *
 * The app used to derive its M3 ColorScheme from `dynamicLightColorScheme`/
 * `dynamicDarkColorScheme` (Material You, wallpaper-seeded). That was the root
 * cause of the web↔app colour drift: every `colorScheme.*` read (buttons,
 * toggles, FAB, icons, the navbar accent) picked up an unpredictable,
 * often-muddy palette and light mode in particular was hard to read.
 *
 * We now seed FIXED light/dark schemes from the shared design tokens in
 * [SpColor.kt] — the same violet accent + slate/gray ramps the web app uses —
 * so the whole Material surface matches the website regardless of the device
 * wallpaper. [LocalSpColors] is still provided alongside for the depth recipes
 * (SpButton etc.).
 */
package com.simplephotos.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.*
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.compose.ui.graphics.Color
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

/**
 * Fixed light scheme — violet accent on a cool-slate neutral ramp, mirroring
 * web `:root` ([SpLightColors]). The surfaceContainer ramp is set explicitly so
 * Cards/sheets land on predictable slate tints instead of the Material baseline
 * purples.
 */
private val LightColorScheme = lightColorScheme(
    primary = Color(0xFF7C3AED),              // violet-600
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFEDE9FE),     // violet-100
    onPrimaryContainer = Color(0xFF4C1D95),   // violet-900
    inversePrimary = Color(0xFFC4B5FD),       // violet-300
    secondary = Color(0xFF5B21B6),            // violet-800
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFEDE9FE),   // violet-100
    onSecondaryContainer = Color(0xFF4C1D95),
    tertiary = Color(0xFF0EA5E9),             // sky-500 (semantic blue accents)
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFFE0F2FE),
    onTertiaryContainer = Color(0xFF0C4A6E),
    background = Color(0xFFF8FAFC),           // slate-50
    onBackground = Color(0xFF0F172A),         // slate-900
    surface = Color(0xFFFFFFFF),              // white
    onSurface = Color(0xFF0F172A),            // slate-900
    surfaceVariant = Color(0xFFF1F5F9),       // slate-100
    onSurfaceVariant = Color(0xFF475569),     // slate-600
    surfaceTint = Color(0xFF7C3AED),
    inverseSurface = Color(0xFF1E293B),       // slate-800
    inverseOnSurface = Color(0xFFF1F5F9),
    error = Color(0xFFDC2626),                // red-600
    onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFFEE2E2),
    onErrorContainer = Color(0xFF7F1D1D),
    outline = Color(0xFFCBD5E1),              // slate-300
    outlineVariant = Color(0xFFE2E8F0),       // slate-200
    scrim = Color(0xFF000000),
    surfaceBright = Color(0xFFFFFFFF),
    surfaceDim = Color(0xFFE2E8F0),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFF8FAFC),  // slate-50
    surfaceContainer = Color(0xFFF1F5F9),     // slate-100
    surfaceContainerHigh = Color(0xFFE9EEF3),
    surfaceContainerHighest = Color(0xFFE2E8F0), // slate-200 (filled Card)
)

/**
 * Fixed dark scheme — lighter violet (reads better on dark) over the original
 * hand-tuned gray ramp, mirroring web `.dark` ([SpDarkColors]).
 */
private val DarkColorScheme = darkColorScheme(
    primary = Color(0xFF8B5CF6),              // violet-500
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFF5B21B6),     // violet-800
    onPrimaryContainer = Color(0xFFEDE9FE),   // violet-100
    inversePrimary = Color(0xFF7C3AED),       // violet-600
    secondary = Color(0xFFA78BFA),            // violet-400
    onSecondary = Color(0xFF2E1065),
    secondaryContainer = Color(0xFF5B21B6),   // violet-800
    onSecondaryContainer = Color(0xFFEDE9FE),
    tertiary = Color(0xFF38BDF8),             // sky-400
    onTertiary = Color(0xFF082F49),
    tertiaryContainer = Color(0xFF075985),
    onTertiaryContainer = Color(0xFFE0F2FE),
    background = Color(0xFF111827),           // gray-900
    onBackground = Color(0xFFF3F4F6),         // gray-100
    surface = Color(0xFF1F2937),             // gray-800
    onSurface = Color(0xFFF3F4F6),            // gray-100
    surfaceVariant = Color(0xFF374151),       // gray-700
    onSurfaceVariant = Color(0xFFD1D5DB),     // gray-300
    surfaceTint = Color(0xFF8B5CF6),
    inverseSurface = Color(0xFFF3F4F6),
    inverseOnSurface = Color(0xFF1F2937),
    error = Color(0xFFF87171),                // red-400 (reads on dark)
    onError = Color(0xFF450A0A),
    errorContainer = Color(0xFF7F1D1D),
    onErrorContainer = Color(0xFFFECACA),
    outline = Color(0xFF4B5563),              // gray-600
    outlineVariant = Color(0xFF374151),       // gray-700
    scrim = Color(0xFF000000),
    surfaceBright = Color(0xFF374151),
    surfaceDim = Color(0xFF111827),
    surfaceContainerLowest = Color(0xFF0B1220),
    surfaceContainerLow = Color(0xFF111827),  // gray-900
    surfaceContainer = Color(0xFF1F2937),     // gray-800
    surfaceContainerHigh = Color(0xFF283344),
    surfaceContainerHighest = Color(0xFF374151), // gray-700 (filled Card)
)

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
    val colorScheme = if (darkTheme) DarkColorScheme else LightColorScheme

    // Provide the shared semantic tokens (violet accent + surface/text ramps)
    // alongside the M3 scheme. The M3 scheme above is now seeded from the same
    // tokens, so Material widgets and the SpButton/SpColors depth recipes agree.
    CompositionLocalProvider(
        LocalSpColors provides if (darkTheme) SpDarkColors else SpLightColors
    ) {
        MaterialTheme(
            colorScheme = colorScheme,
            content = content
        )
    }
}
