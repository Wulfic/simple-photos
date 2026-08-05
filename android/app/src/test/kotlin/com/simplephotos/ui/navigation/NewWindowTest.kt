package com.simplephotos.ui.navigation

import android.content.Intent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * MainActivity is exported, so EXTRA_START_ROUTE is attacker-reachable from any
 * app on the device. [isValidStartRoute] is the only thing standing between a
 * crafted extra and `navController.navigate()`, which makes these the tests that
 * matter most in this file.
 */
class StartRouteValidationTest {

    @Test
    fun `accepts the argument-free browsing routes`() {
        assertTrue(isValidStartRoute("gallery"))
        assertTrue(isValidStartRoute("album_list"))
        assertTrue(isValidStartRoute("search"))
        assertTrue(isValidStartRoute("trash"))
    }

    @Test
    fun `accepts routes built by the Screen helpers`() {
        assertTrue(isValidStartRoute(Screen.PhotoViewer.createRoute("abc123")))
        assertTrue(isValidStartRoute(Screen.PhotoViewer.createRoute("abc123", "smart-recently-added")))
        assertTrue(isValidStartRoute(Screen.AlbumDetail.createRoute("src-9f8e7d")))
        assertTrue(isValidStartRoute(Screen.Gallery.route))
    }

    @Test
    fun `rejects screens a second window has no business opening`() {
        // Secure albums, settings and diagnostics are reachable only from inside
        // an already-unlocked session — never from an intent extra.
        assertFalse(isValidStartRoute("secure_gallery"))
        assertFalse(isValidStartRoute("settings"))
        assertFalse(isValidStartRoute("diagnostics"))
        assertFalse(isValidStartRoute("login"))
        assertFalse(isValidStartRoute("server_setup"))
        assertFalse(isValidStartRoute("two_factor_setup"))
    }

    @Test
    fun `rejects null empty and unknown routes`() {
        assertFalse(isValidStartRoute(null))
        assertFalse(isValidStartRoute(""))
        assertFalse(isValidStartRoute("nonsense"))
        assertFalse(isValidStartRoute("photo_viewer"))
        assertFalse(isValidStartRoute("gallery/extra"))
    }

    @Test
    fun `rejects ids that smuggle extra path segments or query args`() {
        assertFalse(isValidStartRoute("photo_viewer/a/b"))
        assertFalse(isValidStartRoute("photo_viewer/../settings"))
        assertFalse(isValidStartRoute("album_detail/x?albumId=y"))
        assertFalse(isValidStartRoute("photo_viewer/id?albumId=a&evil=b"))
        assertFalse(isValidStartRoute("photo_viewer/id?other=a"))
        assertFalse(isValidStartRoute("photo_viewer/id with spaces"))
        assertFalse(isValidStartRoute("photo_viewer/"))
    }

    @Test
    fun `rejects an over-long id`() {
        assertFalse(isValidStartRoute("photo_viewer/" + "a".repeat(129)))
        assertTrue(isValidStartRoute("photo_viewer/" + "a".repeat(128)))
    }
}

/**
 * The flag combination is the whole mechanic: drop one and you silently get a
 * resumed single window instead of split-screen, with no error anywhere.
 */
class NewWindowFlagsTest {

    @Test
    fun `requests a new task`() {
        // MULTIPLE_TASK and LAUNCH_ADJACENT are both ignored by the platform
        // without NEW_TASK.
        assertNotEquals(0, NEW_WINDOW_FLAGS and Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    @Test
    fun `forces a second task rather than resuming the first`() {
        assertNotEquals(0, NEW_WINDOW_FLAGS and Intent.FLAG_ACTIVITY_MULTIPLE_TASK)
    }

    @Test
    fun `asks to fill the adjacent split-screen pane`() {
        assertNotEquals(0, NEW_WINDOW_FLAGS and Intent.FLAG_ACTIVITY_LAUNCH_ADJACENT)
    }

    @Test
    fun `does not clear or reuse the existing task`() {
        // CLEAR_TOP / SINGLE_TOP would defeat MULTIPLE_TASK by targeting the
        // window that's already open.
        assertEquals(0, NEW_WINDOW_FLAGS and Intent.FLAG_ACTIVITY_CLEAR_TOP)
        assertEquals(0, NEW_WINDOW_FLAGS and Intent.FLAG_ACTIVITY_SINGLE_TOP)
    }
}
