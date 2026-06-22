/**
 * Shared justified-grid row height.
 *
 * Every photo grid in the app (gallery, regular albums, secure albums, search,
 * trash) should size its rows the same way as the primary Photos gallery and
 * honour the Settings → Thumbnail Size preference. Previously each screen
 * hard-coded its own value (180/130/120/110…), so albums/secure/search looked
 * smaller than the main gallery. This centralises it: "large" → 240dp, else
 * 180dp — identical to the primary gallery's rule.
 *
 * It reads the same `settings` DataStore the gallery uses, via a Hilt entry
 * point, so it works inside any composable without threading the setting
 * through every ViewModel.
 */
package com.simplephotos.ui.components

import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.stringPreferencesKey
import dagger.hilt.EntryPoint
import dagger.hilt.InstallIn
import dagger.hilt.android.EntryPointAccessors
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.flow.map

@EntryPoint
@InstallIn(SingletonComponent::class)
internal interface ThumbnailPrefEntryPoint {
    fun dataStore(): DataStore<Preferences>
}

private val KEY_THUMBNAIL_SIZE = stringPreferencesKey("thumbnail_size")

/** Justified-grid target row height honouring the Thumbnail Size setting. */
@Composable
fun rememberGalleryRowHeight(): Dp {
    val context = LocalContext.current
    val dataStore = remember(context) {
        EntryPointAccessors.fromApplication(
            context.applicationContext,
            ThumbnailPrefEntryPoint::class.java
        ).dataStore()
    }
    val size by remember(dataStore) {
        dataStore.data.map { it[KEY_THUMBNAIL_SIZE] ?: "normal" }
    }.collectAsState(initial = "normal")
    return if (size == "large") 240.dp else 180.dp
}
