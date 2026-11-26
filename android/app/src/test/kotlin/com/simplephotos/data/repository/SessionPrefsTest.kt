package com.simplephotos.data.repository

import androidx.datastore.preferences.core.MutablePreferences
import androidx.datastore.preferences.core.mutablePreferencesOf
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_ACCESS_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_BIOMETRIC_ENABLED
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_REFRESH_TOKEN
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_CONFIGURED
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SERVER_URL
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_SESSION_OWNER
import com.simplephotos.ui.navigation.NavViewModel.Companion.KEY_USERNAME
import com.simplephotos.ui.theme.KEY_THEME_MODE
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * N1c. Owner report: after a dead session the only fix was "log out and re-add
 * the server", and re-login then re-encrypted the whole library (N4). An expired
 * session must keep everything but its tokens; logout must keep the server; and
 * a kept mirror must never be shown to a different account.
 */
class SessionPrefsTest {

    private fun loggedIn(): MutablePreferences = mutablePreferencesOf().apply {
        this[KEY_SERVER_URL] = "http://nas:8080"
        this[KEY_SERVER_CONFIGURED] = true
        this[KEY_BIOMETRIC_ENABLED] = true
        SessionPrefs.recordLogin(this, "access-1", "refresh-1", "alice")
    }

    @Test
    fun `expire drops only the tokens`() {
        val prefs = loggedIn()

        assertTrue(SessionPrefs.expire(prefs, "refresh-1"))

        assertNull(prefs[KEY_ACCESS_TOKEN])
        assertNull(prefs[KEY_REFRESH_TOKEN])
        assertEquals("http://nas:8080", prefs[KEY_SERVER_URL])
        assertEquals(true, prefs[KEY_SERVER_CONFIGURED])
        assertEquals("alice", prefs[KEY_USERNAME])
        assertEquals(true, prefs[KEY_BIOMETRIC_ENABLED])
        assertEquals("http://nas:8080|alice", prefs[KEY_SESSION_OWNER])
    }

    @Test
    fun `expire leaves a newer session alone`() {
        // The user logged in again before the dead-session signal was handled.
        val prefs = loggedIn()

        assertFalse(SessionPrefs.expire(prefs, "refresh-0-dead"))

        assertEquals("access-1", prefs[KEY_ACCESS_TOKEN])
        assertEquals("refresh-1", prefs[KEY_REFRESH_TOKEN])
    }

    @Test
    fun `expire with no stored session still routes to login`() {
        // A second window (#21) handling the same signal after the first did.
        val prefs = loggedIn()
        SessionPrefs.expire(prefs, "refresh-1")

        assertTrue(SessionPrefs.expire(prefs, "refresh-1"))
    }

    @Test
    fun `expire pins the owner on an install that never recorded one`() {
        val prefs = loggedIn().apply { remove(KEY_SESSION_OWNER) }

        SessionPrefs.expire(prefs, "refresh-1")
        // Login's "Change server" runs after this; the owner must not follow it.
        prefs[KEY_SERVER_URL] = "http://other:8080"

        assertEquals("http://nas:8080|alice", prefs[KEY_SESSION_OWNER])
        assertTrue(SessionPrefs.mustWipeBeforeLogin(prefs, "alice"))
    }

    @Test
    fun `logout keeps the server and the theme and nothing else`() {
        val prefs = loggedIn().apply { this[KEY_THEME_MODE] = "dark" }

        SessionPrefs.clearForLogout(prefs)

        assertEquals("http://nas:8080", prefs[KEY_SERVER_URL])
        assertEquals(true, prefs[KEY_SERVER_CONFIGURED])
        assertEquals("dark", prefs[KEY_THEME_MODE])
        assertEquals(3, prefs.asMap().size)
    }

    @Test
    fun `same account re-login keeps the mirror`() {
        val prefs = loggedIn()
        SessionPrefs.expire(prefs, "refresh-1")

        assertFalse(SessionPrefs.mustWipeBeforeLogin(prefs, "alice"))
    }

    @Test
    fun `trailing slash on the server url is the same server`() {
        val prefs = loggedIn()
        SessionPrefs.expire(prefs, "refresh-1")
        prefs[KEY_SERVER_URL] = "http://nas:8080/"

        assertFalse(SessionPrefs.mustWipeBeforeLogin(prefs, "alice"))
    }

    @Test
    fun `a different account wipes the mirror first`() {
        val prefs = loggedIn()
        SessionPrefs.expire(prefs, "refresh-1")

        assertTrue(SessionPrefs.mustWipeBeforeLogin(prefs, "bob"))
    }

    @Test
    fun `the same username on a different server wipes the mirror first`() {
        val prefs = loggedIn()
        SessionPrefs.expire(prefs, "refresh-1")
        prefs[KEY_SERVER_URL] = "http://other:8080"

        assertTrue(SessionPrefs.mustWipeBeforeLogin(prefs, "alice"))
    }

    @Test
    fun `an unknown owner fails closed`() {
        val prefs = mutablePreferencesOf().apply { this[KEY_SERVER_URL] = "http://nas:8080" }

        assertTrue(SessionPrefs.mustWipeBeforeLogin(prefs, "alice"))
    }

    @Test
    fun `no configured server fails closed`() {
        assertTrue(SessionPrefs.mustWipeBeforeLogin(mutablePreferencesOf(), "alice"))
    }
}
