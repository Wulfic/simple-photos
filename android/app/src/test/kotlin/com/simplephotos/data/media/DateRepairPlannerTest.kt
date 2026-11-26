package com.simplephotos.data.media

import com.simplephotos.data.media.DateRepairPlanner.PhoneFile
import com.simplephotos.data.media.DateRepairPlanner.ServerRow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * P1c (TODO §4b): which existing backups get a new date. The live library had
 * 5,003 Android rows at 1970-01-01; these pin who is sent, with which compare-and-set
 * token, and who is left alone.
 */
class DateRepairPlannerTest {

    private val now = 1_795_000_000_000L                // 2026-11-17
    private val landedSecs = 1_754_400_000L             // 2025-08-05, MediaStore seconds
    private val landedMs = landedSecs * 1000
    private val exifMs = 1_400_000_000_000L             // 2014-05-13
    private val epoch = "1970-01-01T00:00:00.000Z"

    private fun row(id: String, takenAt: String? = epoch, created: String = "2026-08-05T00:00:00.000Z") =
        ServerRow(id, "$id.jpg", takenAt, created)

    private fun file(
        path: String? = "Download/",
        taken: Long? = null,
        added: Long? = landedSecs,
        modified: Long? = landedSecs,
    ) = PhoneFile(path, taken, added, modified)

    private fun plan(rows: List<ServerRow>, files: Map<String, PhoneFile>) =
        DateRepairPlanner.plan(
            serverRows = rows,
            linkedUri = files.keys.associateBy { it.removePrefix("content://") },
            files = files,
            nowMs = now,
        )

    @Test fun `a dateless download at 1970 moves to the day it landed`() {
        val p = plan(listOf(row("dl")), mapOf("content://dl" to file()))
        val c = p.candidates.single()
        assertEquals("dl", c.photoId)
        assertEquals(landedMs, c.toMs)
        assertEquals(0L, c.fromMs)
        assertEquals(1, p.atEpoch)
    }

    @Test fun `the compare-and-set token is the server's exact string, not Room's`() {
        val p = plan(listOf(row("dl", takenAt = "1970-01-01T00:00:00Z")), mapOf("content://dl" to file()))
        assertEquals("1970-01-01T00:00:00Z", p.candidates.single().expectTakenAt)
    }

    @Test fun `a NULL server date is sent as a null expectation`() {
        val p = plan(listOf(row("n", takenAt = null)), mapOf("content://n" to file()))
        assertNull(p.candidates.single().expectTakenAt)
    }

    @Test fun `a camera photo keeps its capture date`() {
        val exifIso = java.time.Instant.ofEpochMilli(exifMs).toString()
        val p = plan(
            listOf(row("cam", takenAt = exifIso)),
            mapOf("content://cam" to file(path = "DCIM/Camera/", taken = exifMs)),
        )
        assertTrue(p.candidates.isEmpty())
        assertEquals(1, p.alreadyRight)
    }

    @Test fun `the same instant in another format is already right`() {
        val iso = java.time.Instant.ofEpochMilli(landedMs).toString().replace("Z", "+00:00")
        val p = plan(listOf(row("same", takenAt = iso)), mapOf("content://same" to file()))
        assertEquals(1, p.alreadyRight)
        assertTrue(p.candidates.isEmpty())
    }

    @Test fun `an unlinked server row is not on this phone and never guessed`() {
        val p = DateRepairPlanner.plan(
            serverRows = listOf(row("takeout")),
            linkedUri = emptyMap(),
            files = mapOf("content://other" to file()),
            nowMs = now,
        )
        assertEquals(1, p.notOnPhone)
        assertTrue(p.candidates.isEmpty())
    }

    @Test fun `a linked file MediaStore no longer has is gone, not changed`() {
        val p = DateRepairPlanner.plan(
            serverRows = listOf(row("deleted")),
            linkedUri = mapOf("deleted" to "content://deleted"),
            files = emptyMap(),
            nowMs = now,
        )
        assertEquals(1, p.fileGone)
        assertTrue(p.candidates.isEmpty())
    }

    @Test fun `a file with no usable date is skipped, never re-dated to today`() {
        val p = plan(listOf(row("bare")), mapOf("content://bare" to file(added = null, modified = null)))
        assertEquals(1, p.noUsableDate)
        assertTrue(p.candidates.isEmpty())
    }

    @Test fun `a future date is skipped because the server would refuse the chunk`() {
        val futureSecs = (now + 3L * 24 * 60 * 60 * 1000) / 1000
        val p = plan(listOf(row("fut")), mapOf("content://fut" to file(added = futureSecs, modified = futureSecs)))
        assertEquals(1, p.noUsableDate)
    }

    @Test fun `an unparseable server date is skipped because it cannot be a token`() {
        val p = plan(listOf(row("junk", takenAt = "not a date")), mapOf("content://junk" to file()))
        assertEquals(1, p.serverDateUnreadable)
        assertTrue(p.candidates.isEmpty())
    }

    @Test fun `a row repeated across walk pages is sent once`() {
        val p = plan(listOf(row("dup"), row("dup")), mapOf("content://dup" to file()))
        assertEquals(1, p.candidates.size)
    }

    // ── ServerDate ───────────────────────────────────────────────────────

    @Test fun `server date is taken_at, else created_at only when NULL`() {
        assertEquals(0L, ServerDate.ofRecord(epoch, "2026-08-05T00:00:00Z"))
        assertEquals(1_785_888_000_000L, ServerDate.ofRecord(null, "2026-08-05T00:00:00Z"))
        assertNull(ServerDate.ofRecord("garbage", "2026-08-05T00:00:00Z"))
    }

    @Test fun `sync adopts a repaired server date and leaves equal or unreadable ones`() {
        assertEquals(landedMs, ServerDate.adopt(0L, java.time.Instant.ofEpochMilli(landedMs).toString(), "x"))
        assertNull(ServerDate.adopt(0L, "1970-01-01T00:00:00Z", "x"))
        assertNull(ServerDate.adopt(landedMs, "garbage", "2026-08-05T00:00:00Z"))
    }
}
