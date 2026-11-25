package com.simplephotos.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * TODO.md F2b — the upload loop asks the server before it moves any bytes.
 *
 * `ServerHashLookupTest` proves the decision; this proves the loop acts on it.
 * Room cannot run in a JVM test here (no Robolectric), so — as with the D4 and
 * Z1 guards — what is checkable is the source of the loop. A lookup wired in
 * *after* thumbnail generation or the upload would pass every helper test and
 * still send every byte.
 */
class BackupHashLookupWiringTest {

    private val pass: String = run {
        val src = source("com/simplephotos/sync/BackupWorker.kt")
        val start = src.indexOf("private suspend fun runBackupPass()")
        val end = src.indexOf("private fun generateVideoThumbnail(")
        check(start >= 0 && end > start) { "could not isolate the backup pass — the guard would pass vacuously" }
        src.substring(start, end)
    }

    private fun at(needle: String): Int = pass.indexOf(needle).also {
        check(it >= 0) { "'$needle' not found in the backup pass — update this guard with the code" }
    }

    @Test
    fun `the lookup is made once per pass, not once per photo`() {
        assertTrue(at("ServerHashLookup(") < at("for (snapshot in genuinelyPending)"))
    }

    @Test
    fun `the server is asked before a thumbnail is made or a byte is uploaded`() {
        val ask = at("hashLookup.find(contentHash)")
        assertTrue("asked before the hash exists", at("val contentHash =") < ask)
        assertTrue("thumbnail generated before asking", ask < at("val thumbnailData ="))
        assertTrue("chunked upload before asking", ask < at("photoRepository.uploadPhotoChunked("))
        assertTrue("inline upload before asking", ask < at("photoRepository.uploadPhoto("))
    }

    @Test
    fun `a held hash adopts the server photo and skips the upload`() {
        val held = pass.substring(at("is HashLookup.Held")).substringBefore("continue")
        assertTrue("the row must take the server's ids", held.contains("markSynced("))
        assertTrue("held.blobId must be what the row records", held.contains("held.blobId"))
        assertTrue("another row may already draw that photo (D2)", held.contains("collapseTwinsOf(held.photoId)"))
        assertFalse("a held hash must not upload", Regex("""upload\w*\(""").containsMatchIn(held))
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
