package com.simplephotos.ui.navigation

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The whole point of #37 is that Back stops replaying a huge trail of
 * Albums↔Search↔People hops. [shouldJumpToGalleryOnBack] is the pure decision
 * behind that — get the off-by-one or the exemptions wrong and Back either
 * teleports home too early or never stops walking.
 *
 * The stack lists are bottom→top, mirroring `NavController.currentBackStack`,
 * which prepends a null-route graph entry — kept here so the filtering is
 * exercised the way it runs in production.
 */
class BackHistoryTest {

    private val root: String? = null // the NavGraph entry currentBackStack prepends

    /** [root, gallery, then `depth` browsing screens above it]. */
    private fun stackAboveGallery(depth: Int): List<String?> =
        listOf(root, Screen.Gallery.route) +
            (1..depth).map { Screen.AlbumList.route }

    @Test
    fun `walks history at or below the cap`() {
        // Exactly MAX_BACK_HISTORY screens above Gallery — still Back one at a time.
        val stack = stackAboveGallery(MAX_BACK_HISTORY)
        assertFalse(shouldJumpToGalleryOnBack(stack, Screen.AlbumList.route))
    }

    @Test
    fun `jumps home once deeper than the cap`() {
        val stack = stackAboveGallery(MAX_BACK_HISTORY + 1)
        assertTrue(shouldJumpToGalleryOnBack(stack, Screen.AlbumList.route))
    }

    @Test
    fun `does not jump when sitting on the Gallery itself`() {
        // Fresh session: nothing above Gallery, Back should exit the app normally.
        val stack = listOf(root, Screen.Gallery.route)
        assertFalse(shouldJumpToGalleryOnBack(stack, Screen.Gallery.route))
    }

    @Test
    fun `never hijacks Back on screens that own it`() {
        // Deep stack, but the current screen manages its own Back — cap stays off.
        val deep = stackAboveGallery(MAX_BACK_HISTORY + 3)
        for (route in BACK_CAP_EXEMPT_ROUTES) {
            assertFalse(
                "cap must be disabled on $route",
                shouldJumpToGalleryOnBack(deep + route, route),
            )
        }
    }

    @Test
    fun `re-arms on the next non-exempt screen after an exempt pop`() {
        // e.g. deep stack, user backed out of the PhotoViewer onto an album list
        // that is still well past the cap: the cap fires here.
        val stack = stackAboveGallery(MAX_BACK_HISTORY + 2)
        assertTrue(shouldJumpToGalleryOnBack(stack, Screen.AlbumList.route))
    }

    @Test
    fun `does not jump when there is no Gallery to fall back to`() {
        // Pre-login stacks have no Gallery floor — never teleport into one.
        val stack = listOf(root, Screen.Login.route, Screen.Register.route)
        assertFalse(shouldJumpToGalleryOnBack(stack, Screen.Register.route))
    }
}
