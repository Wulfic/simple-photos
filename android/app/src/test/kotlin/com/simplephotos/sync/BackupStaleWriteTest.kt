package com.simplephotos.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * TODO.md D4 — the upload loop wrote whole stale entities back over fresh rows.
 *
 * `db.photoDao().update(photo.copy(photoHash = …))` ran right after a hash-dedup
 * `markSynced`. `photo` was the pass-start snapshot (PENDING, no server ids) and
 * `@Update` replaces every column, so the row it had just synced went back to
 * PENDING. The phone's own logs show the consequence on 2026-11-17: 3,837 rows
 * registered as a copy and then fully re-uploaded by a later pass. The EXIF
 * dimension fix did the same and wiped the thumbnail path saved two lines above.
 *
 * Room cannot run in a JVM unit test here (no Robolectric), so — as with the Z1
 * wiring guards — what is checkable is the source: inside the upload loop, rows
 * are changed by column, never by writing a remembered copy back.
 */
class BackupStaleWriteTest {

    private val uploadLoop: String = run {
        val src = source("com/simplephotos/sync/BackupWorker.kt")
        val start = src.indexOf("private suspend fun runBackupPass()")
        val end = src.indexOf("private fun generateVideoThumbnail(")
        check(start >= 0 && end > start) { "could not isolate the upload loop — the guard would pass vacuously" }
        src.substring(start, end)
    }

    @Test
    fun `the upload loop never writes a whole remembered row back`() {
        assertFalse(
            "update(photo.copy(...)) replaces every column from a stale snapshot — " +
                "this is what reverted synced rows to PENDING",
            Regex("""\.update\(\s*photo\.copy\(""").containsMatchIn(uploadLoop),
        )
        assertFalse(
            "a whole-row update(it) after a copy has the same shape",
            Regex("""\.update\(\s*it\s*\)""").containsMatchIn(uploadLoop),
        )
    }

    @Test
    fun `hash and dimensions are written by column`() {
        assertTrue(uploadLoop.contains("setPhotoHashIfMissing("))
        assertTrue(uploadLoop.contains("updateDimensions("))
    }

    @Test
    fun `a failed copy registration is retried, not marked synced`() {
        // Bounded by the next status write, not by a brace — the log line's
        // `${e.message}` template contains one.
        val failure = uploadLoop.substringAfter("Failed to register hash-dedup copy")
            .substringBefore("++")
        assertTrue(
            "a row marked SYNCED with no server identity is never retried and never " +
                "matches its twin — it stays a duplicate tile forever",
            failure.contains("SyncStatus.FAILED"),
        )
    }

    private fun source(relative: String): String {
        var dir: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (dir != null) {
            for (prefix in listOf("app/src/main/kotlin/", "src/main/kotlin/")) {
                val f = File(dir, prefix + relative)
                if (f.isFile) return f.readText()
            }
            dir = dir.parentFile
        }
        throw AssertionError("could not locate $relative from ${System.getProperty("user.dir")}")
    }
}
