package com.simplephotos.security

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class UnlockSessionTest {

    @Test
    fun `starts locked`() {
        // A fresh instance stands in for a fresh process: a cold start must
        // always re-prompt (#17).
        assertFalse(UnlockSession().isUnlocked)
    }

    @Test
    fun `stays unlocked once the user authenticates`() {
        val session = UnlockSession()
        session.markUnlocked()
        assertTrue(session.isUnlocked)
    }

    @Test
    fun `a second window sees the first window's unlock`() {
        // Both windows are handed the same @Singleton instance, so window #2
        // must not re-prompt seconds after window #1 unlocked (#21).
        val shared = UnlockSession()
        shared.markUnlocked()

        val windowTwoSeesUnlocked = shared.isUnlocked

        assertTrue(windowTwoSeesUnlocked)
    }

    @Test
    fun `marking unlocked twice is idempotent`() {
        val session = UnlockSession()
        session.markUnlocked()
        session.markUnlocked()
        assertTrue(session.isUnlocked)
    }
}
