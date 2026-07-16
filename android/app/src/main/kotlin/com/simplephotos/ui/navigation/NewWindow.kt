/**
 * "Open the app in a second window" — the split-screen mechanic (#21).
 *
 * Instead of a bespoke two-pane viewer, split-screen is a second *instance* of
 * [MainActivity] in its own task. Both instances live in the same process, so
 * they share the Room DB, repositories, Coil cache and auth — the second window
 * is the whole app, with swiping, editing, albums and search, for free.
 *
 * The window is launched with an optional [EXTRA_START_ROUTE] deep-link.
 * MainActivity is exported (it needs the LAUNCHER intent-filter), so ANY app on
 * the device can send us that extra — every route is therefore validated
 * against a whitelist by [isValidStartRoute] before it is ever navigated to.
 */
package com.simplephotos.ui.navigation

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.util.Log
import com.simplephotos.MainActivity

private const val TAG = "NewWindow"

/** Intent extra naming the route the new window should open on. Validated. */
const val EXTRA_START_ROUTE = "com.simplephotos.extra.START_ROUTE"

/**
 * Flags that make a launch open a genuinely separate window rather than
 * resuming the existing one.
 *
 * - `NEW_TASK` — required for the other two to have any effect at all.
 * - `MULTIPLE_TASK` — without it the launcher just brings the existing task
 *   forward and we get one window, not two.
 * - `LAUNCH_ADJACENT` — fills the other split-screen pane. NOTE: on stock
 *   Android this only tiles when the device is *already* in multi-window mode;
 *   from fullscreen it silently just creates the second task and the user snaps
 *   it from Recents (callers surface that hint — see GalleryScreen).
 */
const val NEW_WINDOW_FLAGS = Intent.FLAG_ACTIVITY_NEW_TASK or
    Intent.FLAG_ACTIVITY_MULTIPLE_TASK or
    Intent.FLAG_ACTIVITY_LAUNCH_ADJACENT

/** Routes with no arguments that a second window may open on. */
private val STATIC_START_ROUTES = setOf(
    Screen.Gallery.route,
    Screen.AlbumList.route,
    Screen.Search.route,
    Screen.Trash.route,
)

/**
 * Ids we accept inside a route. Blob ids, local UUIDs and the `smart-` / `src-`
 * album id prefixes all fit; anything with a slash, space or `?` does not, so a
 * crafted extra cannot smuggle extra path segments or query args.
 */
private val ROUTE_ID = Regex("[A-Za-z0-9._~-]{1,128}")

/**
 * Whether [route] is a route a second window is allowed to start on.
 *
 * Deliberately a whitelist, and deliberately smaller than [Screen]: the second
 * window is for *browsing* (gallery, albums, a photo, search, trash). Secure
 * albums, settings, diagnostics and the auth screens are not reachable this way
 * — an exported activity must never navigate wherever a caller asks.
 */
fun isValidStartRoute(route: String?): Boolean {
    if (route.isNullOrEmpty()) return false
    if (route in STATIC_START_ROUTES) return true

    val path = route.substringBefore('?')
    val query = route.substringAfter('?', missingDelimiterValue = "")
    val segments = path.split('/')
    if (segments.size != 2 || !ROUTE_ID.matches(segments[1])) return false

    return when (segments[0]) {
        "album_detail" -> query.isEmpty()
        // photo_viewer/{photoId} optionally carries ?albumId={albumId}, which
        // scopes the viewer's swipe list to that album.
        "photo_viewer" -> query.isEmpty() ||
            (query.startsWith("albumId=") && ROUTE_ID.matches(query.removePrefix("albumId=")))
        else -> false
    }
}

/**
 * The validated start route carried by [intent], or null when there is none or
 * it isn't one we're willing to honor.
 */
fun startRouteFromIntent(intent: Intent?): String? {
    val route = intent?.getStringExtra(EXTRA_START_ROUTE) ?: return null
    if (!isValidStartRoute(route)) {
        Log.e(TAG, "Ignoring start route from intent — not an allowed route: '$route'")
        return null
    }
    return route
}

/**
 * Launch a second window of the app, optionally deep-linked to [route].
 *
 * Returns whether the launch was issued; callers use it to tell the user when
 * nothing happened. A false return is always logged with the cause.
 */
fun openInNewWindow(context: Context, route: String? = null): Boolean {
    if (route != null && !isValidStartRoute(route)) {
        Log.e(TAG, "Refusing to open new window — not an allowed route: '$route'")
        return false
    }
    val intent = Intent(context, MainActivity::class.java).apply {
        addFlags(NEW_WINDOW_FLAGS)
        if (route != null) putExtra(EXTRA_START_ROUTE, route)
    }
    return try {
        context.startActivity(intent)
        Log.i(TAG, "Opened new window (route=${route ?: "default"})")
        true
    } catch (e: Exception) {
        Log.e(TAG, "Failed to open new window (route=${route ?: "default"})", e)
        false
    }
}

/**
 * Unwrap the Activity behind a Compose [Context], or null if there isn't one.
 * Needed to ask `isInMultiWindowMode` — the answer decides whether a second
 * window tiles itself or lands in Recents for the user to arrange.
 */
fun Context.findActivity(): Activity? {
    var context = this
    while (context is ContextWrapper) {
        if (context is Activity) return context
        context = context.baseContext
    }
    return null
}
