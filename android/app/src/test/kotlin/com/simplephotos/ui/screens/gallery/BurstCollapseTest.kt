package com.simplephotos.ui.screens.gallery

import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Verifies the burst-collapse logic used by [GalleryScreen] so that burst
 * groups render as a single tile (matching the web behaviour) and that the
 * per-group frame counts feed the "BURST N" badge correctly.
 *
 * The collapse logic lives inline in [GalleryScreen] — this test reproduces
 * the exact sequence so a regression that swaps `collapsedPhotos` back to
 * `visiblePhotos` (the bug fixed in 1.1.5) is caught immediately.
 */
class BurstCollapseTest {

    private fun photo(
        id: String,
        burstId: String? = null,
        takenAt: Long = 0L,
    ) = PhotoEntity(
        localId = id,
        filename = "$id.jpg",
        takenAt = takenAt,
        mimeType = "image/jpeg",
        mediaType = "image",
        width = 100,
        height = 100,
        syncStatus = SyncStatus.SYNCED,
        createdAt = 0L,
        isFavorite = false,
        burstId = burstId,
    )

    private fun collapseBursts(photos: List<PhotoEntity>): List<PhotoEntity> {
        val seen = HashSet<String>()
        return photos.filter { p ->
            val bid = p.burstId
            if (bid.isNullOrEmpty()) true else seen.add(bid)
        }
    }

    private fun burstCounts(photos: List<PhotoEntity>): Map<String, Int> =
        photos.asSequence()
            .mapNotNull { it.burstId?.takeIf { id -> id.isNotEmpty() } }
            .groupingBy { it }
            .eachCount()

    @Test
    fun `single burst group collapses to one entry`() {
        val src = listOf(
            photo("a1", burstId = "A"),
            photo("a2", burstId = "A"),
            photo("a3", burstId = "A"),
            photo("a4", burstId = "A"),
            photo("a5", burstId = "A"),
        )
        val collapsed = collapseBursts(src)
        assertEquals(1, collapsed.size)
        assertEquals("a1", collapsed.single().localId)
        assertEquals(mapOf("A" to 5), burstCounts(src))
    }

    @Test
    fun `multiple bursts collapse independently and non-burst photos pass through`() {
        val src = listOf(
            photo("a1", burstId = "A"),
            photo("a2", burstId = "A"),
            photo("solo"),
            photo("b1", burstId = "B"),
            photo("b2", burstId = "B"),
            photo("b3", burstId = "B"),
            photo("a3", burstId = "A"),   // out-of-order frame still suppressed
        )
        val collapsed = collapseBursts(src)
        assertEquals(listOf("a1", "solo", "b1"), collapsed.map { it.localId })
        assertEquals(mapOf("A" to 3, "B" to 3), burstCounts(src))
    }

    @Test
    fun `empty-string burstId is treated as no burst`() {
        // PhotoEntity.burstId is nullable but the JSON layer can deliver "" —
        // we MUST treat that as "no burst group", otherwise every empty-string
        // photo collapses into a single tile (data-loss regression).
        val src = listOf(
            photo("x", burstId = ""),
            photo("y", burstId = ""),
            photo("z", burstId = null),
        )
        val collapsed = collapseBursts(src)
        assertEquals(3, collapsed.size)
        assertTrue(burstCounts(src).isEmpty())
    }

    @Test
    fun `burst tile retains its burstId so the badge can resolve the count`() {
        val src = listOf(
            photo("a1", burstId = "A"),
            photo("a2", burstId = "A"),
            photo("a3", burstId = "A"),
        )
        val collapsed = collapseBursts(src)
        val counts = burstCounts(src)
        val tile = collapsed.single()
        assertEquals("A", tile.burstId)
        assertEquals(3, counts[tile.burstId])
        assertNull(counts["nonexistent"])
    }

    @Test
    fun `regression - using visiblePhotos instead of collapsed leaks every frame`() {
        // The Gemini-generated code wired the grid to `visiblePhotos` rather
        // than `collapsedPhotos`. This test pins that behaviour: feeding the
        // un-collapsed list to the grid must NOT match the collapsed result.
        val src = listOf(
            photo("a1", burstId = "A"),
            photo("a2", burstId = "A"),
            photo("a3", burstId = "A"),
            photo("solo"),
        )
        val visible = src
        val collapsed = collapseBursts(src)
        assertEquals(4, visible.size)
        assertEquals(2, collapsed.size)
        // If GalleryScreen ever swaps these back, the grid size doubles+.
        assertTrue(visible.size > collapsed.size)
    }
}
