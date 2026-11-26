/**
 * The preference-level rules for ending a session — pure functions over
 * [MutablePreferences] so they are JVM-testable without a DataStore (N1c).
 */
package com.simplephotos.data.repository

import androidx.datastore.preferences.core.MutablePreferences
import androidx.datastore.preferences.core.Preferences
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_ACCESS_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_REFRESH_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_CONFIGURED
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SESSION_OWNER
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import com.simplephotos.ui.theme.KEY_THEME_MODE

/**
 * Two ways a session ends, deliberately different:
 *  - **expiry** — the server refused the refresh token. The device's library did
 *    not change, so only the tokens go; the same account logging back in finds
 *    its mirror, key and backup folders intact and re-encrypts nothing.
 *  - **logout** — the user asked to leave. Everything goes except device-level
 *    settings: which server this device talks to, and the theme.
 *
 * Keeping a mirror across a re-login is only safe for the same account, so the
 * mirror's owner is recorded and a different account wipes it first.
 */
object SessionPrefs {

    /**
     * Expire the session that [deadRefreshToken] belonged to.
     *
     * Compare-and-clear: tokens are removed only while the stored refresh token
     * still IS the dead one — a login that raced the dead-session signal holds a
     * new token and must not be thrown back to Login.
     *
     * @return true when this device now has no live session (go to Login);
     *   false when a newer session is stored (stay where you are).
     */
    fun expire(prefs: MutablePreferences, deadRefreshToken: String): Boolean {
        val stored = prefs[KEY_REFRESH_TOKEN] ?: return true
        if (stored != deadRefreshToken) return false
        // Pin the owner before the tokens go: installs from before N1c never
        // wrote it, and the Login screen can now change the server, after which
        // KEY_SERVER_URL no longer describes whose mirror this is.
        if (prefs[KEY_SESSION_OWNER] == null) {
            val username = prefs[KEY_USERNAME]
            if (username != null) owner(prefs[KEY_SERVER_URL], username)?.let { prefs[KEY_SESSION_OWNER] = it }
        }
        prefs.remove(KEY_ACCESS_TOKEN)
        prefs.remove(KEY_REFRESH_TOKEN)
        return true
    }

    /**
     * Logout: clear everything except device-level settings — which server this
     * device is set up for, and the theme (Login has its own theme toggle, and a
     * first login on a fresh device runs this same wipe).
     */
    fun clearForLogout(prefs: MutablePreferences) {
        val serverUrl = prefs[KEY_SERVER_URL]
        val configured = prefs[KEY_SERVER_CONFIGURED]
        val theme = prefs[KEY_THEME_MODE]
        prefs.clear()
        serverUrl?.let { prefs[KEY_SERVER_URL] = it }
        configured?.let { prefs[KEY_SERVER_CONFIGURED] = it }
        theme?.let { prefs[KEY_THEME_MODE] = it }
    }

    /** The owner id for [username] on [serverUrl]; null when no server is set. */
    fun owner(serverUrl: String?, username: String): String? =
        serverUrl?.trim()?.trimEnd('/')?.takeIf { it.isNotEmpty() }?.let { "$it|$username" }

    /**
     * Must the local account data be wiped before [username] logs in on the
     * configured server? True unless the mirror is known to be theirs — an
     * unknown owner fails closed.
     */
    fun mustWipeBeforeLogin(prefs: Preferences, username: String): Boolean {
        val incoming = owner(prefs[KEY_SERVER_URL], username) ?: return true
        return prefs[KEY_SESSION_OWNER] != incoming
    }

    /** Record a successful login: tokens, username and the mirror's owner. */
    fun recordLogin(prefs: MutablePreferences, access: String, refresh: String, username: String) {
        prefs[KEY_ACCESS_TOKEN] = access
        prefs[KEY_REFRESH_TOKEN] = refresh
        prefs[KEY_USERNAME] = username
        owner(prefs[KEY_SERVER_URL], username)?.let { prefs[KEY_SESSION_OWNER] = it }
    }
}
