package com.simplephotos.ui.screens.securegallery

import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Mirrors `web/src/gallery/secureMovePicker.test.ts` for the push direction
 * (#43) so both clients move the same items to the same place: whole bursts
 * travel together, items already in the target are skipped, and each item routes
 * from its own source album.
 */
class SecureMovePlanTest {

    private fun item(id: String, galleryId: String?, burstId: String? = null) =
        SecureGalleryItem(id = id, blobId = "blob-$id", addedAt = "", galleryId = galleryId, burstId = burstId)

    private val items = listOf(
        item("a", "g1"),
        item("b1", "g1", burstId = "burst-1"), // representative tile
        item("b2", "g1", burstId = "burst-1"),
        item("b3", "g1", burstId = "burst-1"),
        item("c", "g2"),
    )

    // ── expandBurstSelection ────────────────────────────────────────────────

    @Test
    fun `expands a selected burst to all its frames`() {
        // The grid only exposes the representative "b1".
        assertEquals(
            setOf("b1", "b2", "b3"),
            SecureMovePlan.expandBurstSelection(items, setOf("b1")),
        )
    }

    @Test
    fun `leaves non-burst selections untouched`() {
        assertEquals(
            setOf("a", "c"),
            SecureMovePlan.expandBurstSelection(items, setOf("a", "c")),
        )
    }

    @Test
    fun `mixes burst and non-burst selections`() {
        assertEquals(
            setOf("a", "b1", "b2", "b3"),
            SecureMovePlan.expandBurstSelection(items, setOf("a", "b1")),
        )
    }

    // ── planMovesToTarget ───────────────────────────────────────────────────

    @Test
    fun `routes each item from its own source into the target`() {
        val moves = SecureMovePlan.planMovesToTarget(items, setOf("b1", "c"), "g3")
        assertEquals(
            listOf(
                SecureMovePlan.Move("g1", "b1"),
                SecureMovePlan.Move("g2", "c"),
            ),
            moves,
        )
    }

    @Test
    fun `drops items already in the target album`() {
        // c already lives in g2 — moving the selection into g2 must skip it.
        val moves = SecureMovePlan.planMovesToTarget(items, setOf("b1", "c"), "g2")
        assertEquals(listOf(SecureMovePlan.Move("g1", "b1")), moves)
    }

    @Test
    fun `drops selections with no source gallery`() {
        val orphan = listOf(item("d", null))
        assertEquals(emptyList<SecureMovePlan.Move>(), SecureMovePlan.planMovesToTarget(orphan, setOf("d"), "g1"))
    }

    // ── moveTargets ─────────────────────────────────────────────────────────

    private fun gallery(id: String) = SecureGallery(id = id, name = id.uppercase(), createdAt = "", itemCount = 0)

    @Test
    fun `excludes the open real album`() {
        val targets = SecureMovePlan.moveTargets(listOf(gallery("g1"), gallery("g2"), gallery("g3")), "g2")
        assertEquals(listOf("g1", "g3"), targets.map { it.id })
    }

    @Test
    fun `offers every album for a smart view with a null current id`() {
        val targets = SecureMovePlan.moveTargets(listOf(gallery("g1"), gallery("g2")), null)
        assertEquals(listOf("g1", "g2"), targets.map { it.id })
    }
}
