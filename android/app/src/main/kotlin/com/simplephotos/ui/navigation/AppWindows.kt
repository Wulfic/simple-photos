/**
 * How many app windows are open, and whether another may be opened (#41).
 *
 * Split-screen (#21) works by launching a second *instance* of MainActivity
 * with `FLAG_ACTIVITY_MULTIPLE_TASK`, which spawns a fresh task every single
 * time it is invoked. "New Window" is offered from every `AppHeader`, so
 * nothing stopped a user opening five, ten, twenty windows — all sharing one
 * process, Room DB and Coil cache, which also makes it a memory-pressure
 * contributor to the #51 scroll crash.
 *
 * **Why `ActivityLifecycleCallbacks` and not `onCreate`/`onDestroy` overrides
 * in MainActivity, nor `ActivityManager.appTasks`:**
 *
 * The failure this design has to avoid is a counter that over-counts and never
 * comes back down, because that disables "New Window" permanently — strictly
 * worse than the unbounded windows it was added to prevent.
 *
 * - `appTasks` enumerates *tasks*, not live activities. A task whose activity
 *   the system has reclaimed still appears there, so a backgrounded window the
 *   user can no longer see would keep the cap engaged forever. Self-healing in
 *   appearance only.
 * - Hand-rolled `onCreate`/`onDestroy` overrides pair correctly today but are
 *   one early-return away from leaking, and nothing would catch it.
 * - The framework pairs `onActivityCreated` with `onActivityDestroyed` for
 *   every instance within a process lifetime. The one case where a destroy is
 *   never delivered is the process being killed — and that takes this object
 *   with it, so the count resets to 0 alongside the activities it was counting.
 *   The leak and its cure arrive together.
 *
 * Rotation cannot inflate the count either: MainActivity declares an extensive
 * `android:configChanges` (see AndroidManifest) precisely so configuration
 * changes do NOT destroy and recreate it.
 */
package com.simplephotos.ui.navigation

import android.app.Activity
import android.app.Application
import android.os.Bundle
import android.util.Log
import com.simplephotos.MainActivity
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

private const val TAG = "AppWindows"

/** The cap. Named, not a literal at the comparison site. */
const val MAX_APP_WINDOWS = 2

/**
 * Whether a window may be opened when [live] are already open.
 *
 * Pure, and `>=` rather than `==` on purpose: if the count ever did drift above
 * the cap, an equality test would wave the next window through and keep doing
 * so forever.
 */
fun hasRoomForAnotherWindow(live: Int, max: Int = MAX_APP_WINDOWS): Boolean = live < max

/**
 * Live-window count, as plain Kotlin so it can be driven by a JVM test.
 *
 * Deliberately knows nothing about `Activity` — [AppWindows] supplies that.
 */
class WindowCounter {
    private val _live = MutableStateFlow(0)
    val live: StateFlow<Int> = _live.asStateFlow()

    fun opened() = _live.update { it + 1 }

    /**
     * Clamped at zero. An unbalanced close would otherwise drive the count
     * negative, and a negative count is not a harmless accounting error: it
     * buys extra windows above the cap for every step below zero.
     */
    fun closed() = _live.update { (it - 1).coerceAtLeast(0) }

    fun canOpenAnother(): Boolean = hasRoomForAnotherWindow(_live.value)
}

/** Process-wide window count, fed by the Application's lifecycle callbacks. */
object AppWindows {
    val counter = WindowCounter()

    val live: StateFlow<Int> get() = counter.live

    /** Call once from `Application.onCreate`. */
    fun install(app: Application) {
        app.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {
                if (activity !is MainActivity) return
                counter.opened()
                Log.i(TAG, "Window opened — ${counter.live.value} of $MAX_APP_WINDOWS open")
            }

            override fun onActivityDestroyed(activity: Activity) {
                if (activity !is MainActivity) return
                counter.closed()
                Log.i(TAG, "Window closed — ${counter.live.value} of $MAX_APP_WINDOWS open")
            }

            override fun onActivityStarted(activity: Activity) = Unit
            override fun onActivityResumed(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivityStopped(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
        })
    }
}
