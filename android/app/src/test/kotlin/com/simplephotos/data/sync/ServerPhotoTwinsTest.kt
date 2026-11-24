package com.simplephotos.data.sync

import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import com.simplephotos.data.media.Rendition
import com.simplephotos.data.remote.dto.RegisterEncryptedPhotoResponse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * TODO.md D2 — 4,283 server photos drawn twice on the owner's phone
 * (18,826 Room rows for a ~14.5k-photo library, 2026-11-17).
 */
class ServerPhotoTwinsTest {

    private fun device(
        id: String,
        createdAt: Long = 100,
        blob: String = "uploaded-blob-$id",
        thumbPath: String? = "/thumbs/$id.jpg",
        serverPhotoId: String = "P1",
    ) = PhotoEntity(
        localId = id, serverPhotoId = serverPhotoId, serverBlobId = blob,
        thumbnailBlobId = "uploaded-thumb-$id", filename = "IMG_1.jpg", takenAt = 1L,
        mimeType = "image/jpeg", width = 4032, height = 3024,
        localPath = "content://media/external/images/media/$id", thumbnailPath = thumbPath,
        syncStatus = SyncStatus.SYNCED, createdAt = createdAt, photoHash = "abc123abc123",
    )

    private val rung = Rendition(shortEdge = 1080, width = 1920, height = 1080,
        isSource = false, blobId = "rung-blob", codec = "h264", sizeBytes = 10)

    private fun serverOnly(id: String, createdAt: Long = 50, serverPhotoId: String = "P1") = PhotoEntity(
        localId = id, serverPhotoId = serverPhotoId, serverBlobId = "canonical-blob",
        thumbnailBlobId = "canonical-thumb", filename = "IMG_1.jpg", takenAt = 1L,
        mimeType = "image/jpeg", width = 4032, height = 3024, localPath = null,
        thumbnailPath = "/thumbs/$id.jpg", syncStatus = SyncStatus.SYNCED,
        createdAt = createdAt, isFavorite = true, cropMetadata = """{"rotate":90}""",
        photoSubtype = "motion", motionVideoBlobId = "mv-blob", renditions = listOf(rung),
        sizeBytes = 2_000_000,
    )

    @Test
    fun `one row is not a twin`() {
        assertNull(planTwinCollapse(emptyList()))
        assertNull(planTwinCollapse(listOf(device("a"))))
    }

    @Test
    fun `the live shape - device row plus the server-only row the sync inserted`() {
        val plan = planTwinCollapse(listOf(serverOnly("s"), device("d")))!!
        assertEquals("the device row survives — it is the one the backup tracks", "d", plan.keeper.localId)
        assertEquals(listOf("s"), plan.victims.map { it.localId })
        // The keeper adopts what the SERVER serves, not the blob it uploaded and the
        // server discarded — that blob is what slipped past the secure filter (S1).
        assertEquals("canonical-blob", plan.keeper.serverBlobId)
        assertEquals("canonical-thumb", plan.keeper.thumbnailBlobId)
        assertTrue(plan.keeper.isFavorite)
        assertEquals("""{"rotate":90}""", plan.keeper.cropMetadata)
        assertEquals("motion", plan.keeper.photoSubtype)
        assertEquals("mv-blob", plan.keeper.motionVideoBlobId)
        assertEquals(listOf(rung), plan.keeper.renditions)
        assertEquals(2_000_000L, plan.keeper.sizeBytes)
        // …and keeps what only the device knows.
        assertEquals("content://media/external/images/media/d", plan.keeper.localPath)
        assertEquals("/thumbs/d.jpg", plan.keeper.thumbnailPath)
        assertEquals("abc123abc123", plan.keeper.photoHash)
        assertEquals(SyncStatus.SYNCED, plan.keeper.syncStatus)
        assertEquals(listOf("/thumbs/s.jpg"), plan.orphanedThumbnails)
    }

    @Test
    fun `two device copies of one file keep the oldest and their own server state`() {
        // DCIM and DCIM/Restored both backed up — no server-only twin to copy from.
        val plan = planTwinCollapse(listOf(device("new", createdAt = 200), device("old", createdAt = 100)))!!
        assertEquals("old", plan.keeper.localId)
        assertEquals("uploaded-blob-old", plan.keeper.serverBlobId)
        assertEquals(listOf("new"), plan.victims.map { it.localId })
        assertEquals(listOf("/thumbs/new.jpg"), plan.orphanedThumbnails)
    }

    @Test
    fun `the choice is stable whatever order Room returns the rows in`() {
        // An unstable keeper would flip every pass and churn thumbnails.
        val rows = listOf(device("b", createdAt = 100), device("a", createdAt = 100), serverOnly("s"))
        val keepers = listOf(rows, rows.reversed(), listOf(rows[2], rows[0], rows[1]))
            .map { planTwinCollapse(it)!!.keeper.localId }
            .toSet()
        assertEquals(setOf("a"), keepers)
    }

    @Test
    fun `server-only twins alone collapse to the oldest`() {
        val plan = planTwinCollapse(listOf(serverOnly("s2", createdAt = 90), serverOnly("s1", createdAt = 10)))!!
        assertEquals("s1", plan.keeper.localId)
        assertEquals(listOf("s2"), plan.victims.map { it.localId })
    }

    @Test
    fun `a keeper with no cached thumbnail borrows its twin's instead of losing it`() {
        val plan = planTwinCollapse(listOf(serverOnly("s"), device("d", thumbPath = null)))!!
        assertEquals("/thumbs/s.jpg", plan.keeper.thumbnailPath)
        assertTrue("a file the keeper now uses must not be deleted", plan.orphanedThumbnails.isEmpty())
    }

    @Test
    fun `the keeper is never among the victims`() {
        val rows = listOf(device("a"), device("b", createdAt = 300), serverOnly("s"), serverOnly("t", createdAt = 60))
        val plan = planTwinCollapse(rows)!!
        assertFalse(plan.victims.any { it.localId == plan.keeper.localId })
        assertEquals(rows.size - 1, plan.victims.size)
    }

    @Test(expected = IllegalArgumentException::class)
    fun `rows of DIFFERENT server photos are refused, not merged`() {
        // Fail closed: merging two real photos would delete a tile the user should see.
        planTwinCollapse(listOf(device("a", serverPhotoId = "P1"), device("b", serverPhotoId = "P2")))
    }

    @Test(expected = IllegalArgumentException::class)
    fun `rows with no server identity are refused`() {
        planTwinCollapse(listOf(
            device("a").copy(serverPhotoId = null),
            device("b").copy(serverPhotoId = null),
        ))
    }

    // ── canonicalBlobIds ─────────────────────────────────────────────────

    @Test
    fun `a duplicate answer that names the canonical blobs wins`() {
        val res = RegisterEncryptedPhotoResponse("P1", duplicate = true,
            encryptedBlobId = "canon", encryptedThumbBlobId = "canon-thumb")
        assertEquals("canon" to "canon-thumb", canonicalBlobIds(res, "mine", "my-thumb"))
    }

    @Test
    fun `an older server that names nothing keeps the uploaded pair`() {
        val res = RegisterEncryptedPhotoResponse("P1", duplicate = true)
        assertEquals("mine" to "my-thumb", canonicalBlobIds(res, "mine", "my-thumb"))
    }

    @Test
    fun `a new photo keeps the uploaded pair`() {
        val res = RegisterEncryptedPhotoResponse("P1", duplicate = null,
            encryptedBlobId = "ignored", encryptedThumbBlobId = "ignored")
        assertEquals("mine" to "my-thumb", canonicalBlobIds(res, "mine", "my-thumb"))
    }

    @Test
    fun `a canonical photo with no thumbnail keeps ours rather than none`() {
        val res = RegisterEncryptedPhotoResponse("P1", duplicate = true, encryptedBlobId = "canon")
        assertEquals("canon" to "my-thumb", canonicalBlobIds(res, "mine", "my-thumb"))
    }

    // ── Wiring guards: a planner nobody calls is the Z1 failure shape ────

    @Test
    fun `the sync heals twins before the steady-state shortcut can skip it`() {
        val src = source("com/simplephotos/data/repository/PhotoRepository.kt")
        val sync = src.substringAfter("suspend fun syncFromServer(): Int {")
        val heal = sync.indexOf("healTwinsOncePerProcess()")
        val skip = sync.indexOf("if (mode == SyncMode.SKIPPED)")
        assertTrue("syncFromServer must heal twins", heal >= 0)
        assertTrue("…before the SKIPPED early return, or a steady-state phone never heals", heal in 0 until skip)
    }

    @Test
    fun `register records the canonical blobs and collapses last`() {
        val src = source("com/simplephotos/data/repository/PhotoRepository.kt")
        val reg = src.substringAfter("private suspend fun registerAndMark(").substringBefore("suspend fun downloadMotionVideoToFile")
        assertTrue(reg.contains("canonicalBlobIds("))
        val collapse = reg.indexOf("collapseTwinsOf(")
        assertTrue("registerAndMark must collapse twins on a duplicate", collapse >= 0)
        assertTrue("…after the thumbnail write, which a collapse may orphan",
            collapse > reg.indexOf("saveThumbnailToDisk("))
    }

    @Test
    fun `the backup dedup path records canonical blobs and collapses`() {
        val src = source("com/simplephotos/sync/BackupWorker.kt")
        val dedup = src.substringAfter("val regRes = api.registerEncryptedPhoto(regReq)").substringBefore("uploaded++")
        assertTrue(dedup.contains("canonicalBlobIds("))
        assertTrue(dedup.contains("collapseTwinsOf("))
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
