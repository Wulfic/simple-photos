/**
 * Bounded back-history for the hardware/gesture Back button (#37).
 *
 * The nav stack grows one entry per hop and — because top-level destinations
 * are reached from a drawer on almost every screen — a real session easily
 * stacks a dozen Albums↔Search↔People hops. Plain Back then replays every one
 * of them before the user ever reaches somewhere useful.
 *
 * The wanted behaviour: Back walks the genuine history for a few screens, and
 * once you are deeper than [MAX_BACK_HISTORY] screens above the Gallery it
 * short-circuits straight home instead of unwinding the whole trail. Gallery is
 * always the bottom of the stack for a logged-in session (every "home"
 * navigation does `popUpTo(0)`), so it is the natural floor to measure against
 * and the safe place to jump to.
 *
 * The decision is a pure function of the current route stack so it is unit
 * testable without a NavController — see BackHistoryTest.
 */
package com.simplephotos.ui.navigation

/**
 * How many screens above the Gallery we let Back walk one-at-a-time before it
 * jumps straight home. "up to 5, then Gallery" from the issue.
 */
const val MAX_BACK_HISTORY = 5

/**
 * Routes where Back has its own meaning and the cap must never hijack it:
 *
 * - PhotoViewer — Back exits the viewer (and, in edit mode, the edit) back to
 *   the screen that opened it. Jumping home from here would strand the user.
 * - SecureGallery — Back leaves the (unlocked) secure area / dismisses its
 *   password gate; it manages its own exit.
 * - the auth screens — Back inside login/register/setup must not teleport into
 *   an authenticated Gallery.
 *
 * On these screens the cap is disabled and the system performs a normal pop,
 * which walks the user one screen shallower; the very next non-exempt screen
 * re-arms the cap if the stack is still too deep.
 */
val BACK_CAP_EXEMPT_ROUTES: Set<String> = setOf(
    Screen.PhotoViewer.route,
    Screen.SecureGallery.route,
    Screen.ServerSetup.route,
    Screen.Login.route,
    Screen.Register.route,
)

/**
 * Whether a Back press on [currentRoute], with the given bottom→top [routeStack]
 * (as reported by `NavController.currentBackStack`, nulls for graph entries
 * allowed), should jump straight to the Gallery instead of popping one screen.
 *
 * True only when all of:
 *  - the current screen isn't one that owns its own Back ([BACK_CAP_EXEMPT_ROUTES]),
 *  - the Gallery is actually on the stack to fall back to, and
 *  - there are more than [MAX_BACK_HISTORY] screens stacked above that Gallery.
 */
fun shouldJumpToGalleryOnBack(routeStack: List<String?>, currentRoute: String?): Boolean {
    if (currentRoute in BACK_CAP_EXEMPT_ROUTES) return false
    val screens = routeStack.filterNotNull()
    val galleryIndex = screens.indexOf(Screen.Gallery.route)
    if (galleryIndex < 0) return false
    val depthAboveGallery = screens.size - galleryIndex - 1
    return depthAboveGallery > MAX_BACK_HISTORY
}
