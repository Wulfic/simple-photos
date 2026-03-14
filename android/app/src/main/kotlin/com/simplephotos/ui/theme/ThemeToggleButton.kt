/**
 * Light/dark mode toggle icon button composable.
 *
 * Reads the current theme from [ThemeState] and toggles between light and
 * dark modes on click, persisting the choice to DataStore.
 */
package com.simplephotos.ui.theme

import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import com.simplephotos.R

@Composable
fun ThemeToggleButton(
    dataStore: DataStore<Preferences>,
    modifier: Modifier = Modifier
) {
    val systemDark = androidx.compose.foundation.isSystemInDarkTheme()
    val isDark = ThemeState.isDark(systemDark)

    IconButton(
        onClick = { ThemeState.toggle(dataStore, isDark) },
        modifier = modifier
    ) {
        Icon(
            painter = painterResource(if (isDark) R.drawable.ic_sun else R.drawable.ic_night),
            contentDescription = if (isDark) "Switch to light mode" else "Switch to dark mode",
            modifier = Modifier.size(24.dp),
            tint = MaterialTheme.colorScheme.onSurface
        )
    }
}
