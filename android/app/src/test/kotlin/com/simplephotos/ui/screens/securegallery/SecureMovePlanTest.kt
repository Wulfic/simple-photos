package com.simplephotos.ui.screens.securegallery

import com.simplephotos.data.remote.dto.SecureGallery
import com.simplephotos.data.remote.dto.SecureGalleryItem
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Mirrors `web/src/gallery/secureMovePicker.test.ts` so both clients file the
 * same items into the same place: whole bursts travel together, one request per
 * clone, and — since Z1 — the push direction **adds** rather than moves, so
 * nothing is dropped for "already in the target" that only the server can judge.
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

    // ── planAddsToTarget (Z1: the push direction ADDS, it does not move) ─────

    @Test
    fun `plans one add per selected item, keyed on the clone blob`() {
        val adds = SecureMovePlan.planAddsToTarget(items, setOf("a", "c"))
        assertEquals(
            listOf(
                SecureMovePlan.Add("a", "blob-a"),
                SecureMovePlan.Add("c", "blob-c"),
            ),
            adds,
        )
    }

    @Test
    fun `does NOT drop an item whose own album is the target`() {
        // The move planner dropped these; an add cannot, because "is it already
        // in the target" lives in a different album's membership rows that this
        // feed cannot see. The server answers it with a 409. Guessing here would
        // be a second derivation of membership from a feed that cannot see the
        // answer — and it would silently skip a legitimate add.
        val adds = SecureMovePlan.planAddsToTarget(items, setOf("c"))
        assertEquals(listOf(SecureMovePlan.Add("c", "blob-c")), adds)
    }

    @Test
    fun `issues one add per clone when several selected frames share one`() {
        // Two burst frames sharing a clone would otherwise issue two requests,
        // the second guaranteed to 409.
        val shared = listOf(
            item("f1", "g1").copy(blobId = "clone-1"),
            item("f2", "g1").copy(blobId = "clone-1"),
        )
        val adds = SecureMovePlan.planAddsToTarget(shared, setOf("f1", "f2"))
        assertEquals(listOf(SecureMovePlan.Add("f1", "clone-1")), adds)
    }

    @Test
    fun `drops selections that are not in the pool`() {
        assertEquals(
            emptyList<SecureMovePlan.Add>(),
            SecureMovePlan.planAddsToTarget(items, setOf("not-here")),
        )
    }

    // ── expandForRemoval ────────────────────────────────────────────────────

    @Test
    fun `a removal of a burst cover carries every frame`() {
        val expanded = SecureMovePlan.expandForRemoval(items, listOf(items[1]))
        assertEquals(setOf("b1", "b2", "b3"), expanded.map { it.id }.toSet())
    }

    @Test
    fun `a removal of a plain item carries only itself`() {
        val expanded = SecureMovePlan.expandForRemoval(items, listOf(items[0]))
        assertEquals(listOf("a"), expanded.map { it.id })
    }

    @Test
    fun `burst siblings must share the owning album, not just the burst id`() {
        // Guards a cross-album burst_id collision: pulling a stranger's frame in
        // would remove a photo from an album the user never touched.
        val crossAlbum = items + item("x", "g2", burstId = "burst-1")
        val expanded = SecureMovePlan.expandForRemoval(crossAlbum, listOf(crossAlbum[1]))
        assertEquals(setOf("b1", "b2", "b3"), expanded.map { it.id }.toSet())
    }

    @Test
    fun `a target listed twice is removed once`() {
        val expanded = SecureMovePlan.expandForRemoval(items, listOf(items[1], items[2]))
        assertEquals(3, expanded.size)
    }

    // ── addTargets ──────────────────────────────────────────────────────────

    private fun gallery(id: String) = SecureGallery(id = id, name = id.uppercase(), createdAt = "", itemCount = 0)

    @Test
    fun `excludes the open real album`() {
        val targets = SecureMovePlan.addTargets(listOf(gallery("g1"), gallery("g2"), gallery("g3")), "g2")
        assertEquals(listOf("g1", "g3"), targets.map { it.id })
    }

    @Test
    fun `offers every album for a smart view with a null current id`() {
        val targets = SecureMovePlan.addTargets(listOf(gallery("g1"), gallery("g2")), null)
        assertEquals(listOf("g1", "g2"), targets.map { it.id })
    }
}
