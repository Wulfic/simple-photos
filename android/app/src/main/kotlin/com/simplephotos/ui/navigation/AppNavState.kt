/**
 * App-wide navigation + identity surfaced to detail screens so they can render
 * the shared [com.simplephotos.ui.components.AppHeader] navbar (#35 — keep the
 * main navbar visible across the albums/library section) without threading the
 * eight navigation callbacks + username through every route in [NavGraph].
 *
 * Provided once around the NavHost in [NavGraph]; `null` until then, so readers
 * must null-guard (only the deep-linked start window can compose a screen before
 * it is set).
 */
package com.simplephotos.ui.navigation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import com.simplephotos.ui.components.ActiveTab
import com.simplephotos.ui.components.AppHeader
import com.simplephotos.ui.components.HeaderNavigation

data class AppNavState(
    val username: String,
    val navigation: HeaderNavigation,
)

val LocalAppNav = staticCompositionLocalOf<AppNavState?> { null }

/**
 * The shared app navbar for album/library detail screens (#35), sourced from
 * [LocalAppNav]. Renders nothing until the provider is set (deep-link start
 * window edge case). Highlights the Albums tab by default since every screen
 * that uses it lives under the albums section.
 */
@Composable
fun DetailNavBar(activeTab: ActiveTab = ActiveTab.ALBUMS) {
    val nav = LocalAppNav.current ?: return
    AppHeader(activeTab = activeTab, username = nav.username, navigation = nav.navigation)
}
