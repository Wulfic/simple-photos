package com.simplephotos.data.media

import com.simplephotos.data.media.DateRepairPlanner.Candidate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * P2 (TODO §4c): the date repair now runs unattended after backups. These pin the
 * two things that make that safe: it runs rarely, and it can only ever replace a
 * 1970 value — never a date someone chose.
 */
class AutoDateRepairTest {

    private val now = 1_795_000_000_000L
    private val hour = 60L * 60 * 1000

    private fun cand(expect: String?) = Candidate("p", "p.jpg", expect, 0L, 1_754_400_000_000L)

    @Test fun `runs the first time and after the interval, not before`() {
        assertTrue(AutoDateRepair.isDue(null, now))
        assertFalse(AutoDateRepair.isDue(now - 5 * hour, now))
        assertTrue(AutoDateRepair.isDue(now - 6 * hour, now))
    }

    @Test fun `a last-run stamp in the future does not disable it forever`() {
        assertTrue(AutoDateRepair.isDue(now + 24 * hour, now))
    }

    @Test fun `only candidates replacing a 1970 value are kept`() {
        val (kept, dropped) = AutoDateRepair.epochOnly(listOf(
            cand("1970-01-01T00:00:00.000Z"),
            cand("1970-01-01T00:00:00Z"),
            cand("1970-01-01T23:59:59.999Z"),
            cand("2023-04-01T00:00:00Z"),   // someone's real or hand-set date
            cand("1970-01-02T00:00:00Z"),
            cand(null),                      // NULL column: not the bug value
            cand("garbage"),
        ))
        assertEquals(3, kept.size)
        assertEquals(4, dropped)
    }
}
