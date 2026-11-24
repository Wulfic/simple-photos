package com.simplephotos.sync

import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.yield
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * TODO.md D1: three differently-named WorkManager requests ran backup passes side
 * by side on the same pending rows (5,327 stale re-registrations on 2026-11-17).
 */
class BackupRunGateTest {

    @Test
    fun `a second pass is refused while the first is running`() = runBlocking {
        val gate = SingleFlight()
        val firstEntered = CompletableDeferred<Unit>()
        val releaseFirst = CompletableDeferred<Unit>()
        val first = async {
            gate.runExclusive {
                firstEntered.complete(Unit)
                releaseFirst.await()
                "first"
            }
        }
        firstEntered.await()
        var secondRan = false
        val second = gate.runExclusive { secondRan = true; "second" }
        assertNull("a concurrent pass must be refused, not queued", second)
        assertFalse("the refused block must never execute", secondRan)
        releaseFirst.complete(Unit)
        assertEquals("first", first.await())
    }

    @Test
    fun `the gate reopens after a pass finishes`() = runBlocking {
        val gate = SingleFlight()
        assertEquals(1, gate.runExclusive { 1 })
        assertEquals(2, gate.runExclusive { 2 })
        assertFalse(gate.isRunning)
    }

    @Test
    fun `a pass that throws does not wedge the gate shut`() = runBlocking {
        // A wedged gate would silently stop ALL future backups — worse than the
        // duplicates it exists to prevent.
        val gate = SingleFlight()
        try {
            gate.runExclusive { throw IllegalStateException("upload blew up") }
        } catch (_: IllegalStateException) {
            // expected — rethrown to the caller unchanged
        }
        assertFalse(gate.isRunning)
        assertEquals("ok", gate.runExclusive { "ok" })
    }

    @Test
    fun `a cancelled pass does not wedge the gate shut`() = runBlocking {
        // WorkManager cancels workers (constraints lost, Doze); that must release too.
        val gate = SingleFlight()
        val entered = CompletableDeferred<Unit>()
        val job = launch {
            gate.runExclusive {
                entered.complete(Unit)
                CompletableDeferred<Unit>().await() // never completes
            }
        }
        entered.await()
        assertTrue(gate.isRunning)
        job.cancelAndJoin()
        yield()
        assertFalse(gate.isRunning)
        assertEquals("ok", gate.runExclusive { "ok" })
    }

    private fun row(
        status: SyncStatus,
        serverPhotoId: String? = null,
        serverBlobId: String? = null,
    ) = PhotoEntity(
        localId = "l1", filename = "IMG_1.jpg", takenAt = 0L, mimeType = "image/jpeg",
        width = 1, height = 1, localPath = "content://media/1", syncStatus = status,
        serverPhotoId = serverPhotoId, serverBlobId = serverBlobId,
    )

    @Test
    fun `only a row that is still pending and unregistered is uploaded`() {
        assertTrue(stillNeedsUpload(row(SyncStatus.PENDING)))
        assertTrue(stillNeedsUpload(row(SyncStatus.FAILED)))
        assertFalse("gone (purged or collapsed) — nothing to upload", stillNeedsUpload(null))
        assertFalse("another uploader is mid-flight", stillNeedsUpload(row(SyncStatus.UPLOADING)))
        assertFalse(stillNeedsUpload(row(SyncStatus.SYNCED)))
        // The exact stale-snapshot shape from the 11-17 logs: the snapshot still
        // said PENDING while Room already held the server identity.
        assertFalse(stillNeedsUpload(row(SyncStatus.PENDING, serverPhotoId = "p1")))
        assertFalse(stillNeedsUpload(row(SyncStatus.FAILED, serverBlobId = "b1")))
    }

    /** Wiring guard — a gate with no call site is the Z1 "tested, wired into nothing" shape. */
    @Test
    fun `BackupWorker runs every pass through the gate and re-reads each row`() {
        val src = source("com/simplephotos/sync/BackupWorker.kt")
        val doWork = src.substringAfter("override suspend fun doWork()").substringBefore("private suspend fun runBackupPass()")
        assertTrue("doWork must go through the single-flight gate", doWork.contains("backupGate.runExclusive"))
        assertTrue("a skipped run must re-arm the reactive trigger", doWork.contains("rescheduleReactive"))
        assertTrue("the gate must be process-wide (companion), not per-instance",
            src.substringAfter("companion object").substringBefore("override suspend fun getForegroundInfo")
                .contains("SingleFlight()"))
        assertTrue("each row must be re-read before upload", src.contains("stillNeedsUpload("))
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
