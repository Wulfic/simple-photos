package com.simplephotos.ui.navigation

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The two-window cap (#41).
 *
 * `WindowCounter` is deliberately free of `Activity` so these can run on the
 * JVM — the Android half of [AppWindows] is only the lifecycle-callback glue
 * that calls `opened()` / `closed()`.
 *
 * The failure mode being guarded against is NOT "too many windows open". It is
 * a count that drifts and never recovers, because that disables "New Window"
 * permanently — worse than the unbounded windows the cap was added to prevent.
 */
class WindowCounterTest {

    @Test
    fun `allows a second window and refuses a third`() {
        val counter = WindowCounter()
        assertTrue("no windows open — must allow one", counter.canOpenAnother())

        counter.opened()
        assertTrue("one window open — split screen needs a second", counter.canOpenAnother())

        counter.opened()
        assertFalse("two windows open — this is the cap", counter.canOpenAnother())
        assertEquals(2, counter.live.value)
    }

    @Test
    fun `closing a window frees its slot again`() {
        val counter = WindowCounter()
        counter.opened(); counter.opened()
        assertFalse(counter.canOpenAnother())

        counter.closed()
        assertTrue("closing a window must re-enable the action", counter.canOpenAnother())
        assertEquals(1, counter.live.value)
    }

    @Test
    fun `an unbalanced close cannot drive the count negative`() {
        // A negative count is not a harmless accounting error — every step
        // below zero buys one extra window above the cap.
        val counter = WindowCounter()
        counter.closed(); counter.closed(); counter.closed()
        assertEquals(0, counter.live.value)

        counter.opened(); counter.opened()
        assertFalse("the cap must still bite after an unbalanced close", counter.canOpenAnother())
    }

    @Test
    fun `a count somehow above the cap still refuses, and recovers as windows close`() {
        // `hasRoomForAnotherWindow` uses `<`, not `!=`. Were it an equality
        // test, a count that overshot would wave every subsequent window
        // through forever.
        assertFalse(hasRoomForAnotherWindow(3))
        assertFalse(hasRoomForAnotherWindow(99))
        // And it is not a one-way door: the cap lifts again on the way down.
        assertTrue(hasRoomForAnotherWindow(1))
        assertTrue(hasRoomForAnotherWindow(0))
    }

    @Test
    fun `the cap is two`() {
        // Pinned because the toast text says "Already using 2 windows" and the
        // issue asks for exactly two.
        assertEquals(2, MAX_APP_WINDOWS)
        assertTrue(hasRoomForAnotherWindow(MAX_APP_WINDOWS - 1))
        assertFalse(hasRoomForAnotherWindow(MAX_APP_WINDOWS))
    }

    @Test
    fun `live count is observable so a menu can grey itself out`() {
        // The UI disables the entry from this flow rather than discovering the
        // cap on tap, so the flow has to actually track the count.
        val counter = WindowCounter()
        val seen = mutableListOf<Int>()
        seen += counter.live.value

        counter.opened(); seen += counter.live.value
        counter.opened(); seen += counter.live.value
        counter.closed(); seen += counter.live.value

        assertEquals(listOf(0, 1, 2, 1), seen)
    }
}
