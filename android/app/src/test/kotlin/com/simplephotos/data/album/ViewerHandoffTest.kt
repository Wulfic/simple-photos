package com.simplephotos.data.album

import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pins the non-album viewer handoff (#52, E3a) — [gridPhotoIds] and
 * [orderPhotosBy].
 *
 * Two defects are asserted here, and the first one is the reason this file
 * exists rather than a couple of extra cases in `AlbumPhotoResolverTest`:
 *
 *  1. **The id space.** People / Pets / Memories / Trips listed *server* photo
 *     ids and handed them to a viewer that locates photos by `localId`. Both are
 *     strings, both look like ids, and nothing anywhere failed loudly — the
 *     viewer simply never found the photo. [aServerIdIsNeverALocalId] states the
 *     invariant that makes every other case in this file meaningful.
 *  2. **The order.** The handed-over list must survive the Room `IN (...)`
 *     lookup, which answers in arbitrary order. Re-sorting it would be a second
 *     derivation of the list — the defect E3 removed.
 */
class ViewerHandoffTest {

    private fun photo(
        localId: String,
        serverPhotoId: String? = null,
        takenAt: Long = 0L,
    ) = PhotoEntity(
        localId = localId,
        serverPhotoId = serverPhotoId,
        filename = "$localId.jpg",
        takenAt = takenAt,
        mimeType = "image/jpeg",
        mediaType = "image",
        width = 100,
        height = 100,
        syncStatus = SyncStatus.SYNCED,
        createdAt = takenAt,
        isFavorite = false,
        serverBlobId = localId,
    )

    // ── Defect 1: server ids were handed to a localId lookup ────────────────

    @Test
    fun `a server id is never a local id`() {
        // The invariant the whole bug rests on. PhotoRepository.buildSyncedEntity
        // assigns localId = UUID.randomUUID(); the server id is a separate value
        // stored alongside it. The four library grids passed the latter where the
        // viewer read the former, so the lookup could not match — ever.
        val mirror = listOf(photo(localId = "local-uuid-1", serverPhotoId = "srv-1"))
        assertNotEquals(mirror[0].localId, mirror[0].serverPhotoId)

        val grid = gridPhotoIds(listOf("srv-1"), mirror)

        // What the four screens used to navigate with vs. what they must now.
        assertEquals("local-uuid-1", grid.viewerIdFor("srv-1"))
        assertNotEquals("srv-1", grid.viewerIdFor("srv-1"))
    }

    @Test
    fun `the pre-fix navigation id resolved to no page at all`() {
        // Reproduces the whole broken path end to end: grid lists server ids →
        // navigate with the server id → resolver pages local ids. Before E3 this
        // -1 was coerced to 0 and the viewer opened an unrelated photo; after E3
        // it rendered "Photo not found". Either way the user never saw the photo
        // they tapped.
        val mirror = listOf(
            photo(localId = "local-a", serverPhotoId = "srv-a"),
            photo(localId = "local-b", serverPhotoId = "srv-b"),
        )
        val grid = gridPhotoIds(listOf("srv-a", "srv-b"), mirror)
        val resolved = resolvePhotos(
            members = orderPhotosBy(grid.viewerIds, mirror),
            secureBlobIds = emptySet(),
            sort = null,
            collapseBurstStacks = false,
        )

        assertEquals(-1, resolved.pageIndexOf("srv-a"))            // the old id
        assertEquals(0, resolved.pageIndexOf(grid.viewerIdFor("srv-a"))) // the new one
    }

    // ── Defect 2: the grid's order is the thing being preserved ─────────────

    @Test
    fun `the grid's order survives the mirror lookup's arbitrary order`() {
        // Room's `IN (...)` makes no ordering promise. A face cluster, a trip and
        // a relevance-ranked search each have an order the viewer cannot rebuild
        // from the photos themselves, so it has to come across intact.
        val mirror = listOf(
            photo(localId = "c", takenAt = 100),
            photo(localId = "a", takenAt = 900),
            photo(localId = "b", takenAt = 500),
        )
        val requested = listOf("b", "c", "a")

        assertEquals(requested, orderPhotosBy(requested, mirror).map { it.localId })
        // Explicitly NOT the gallery's takenAt DESC, which is what the viewer
        // fell back to before the handoff existed.
        assertNotEquals(
            listOf("a", "b", "c"),
            orderPhotosBy(requested, mirror).map { it.localId },
        )
    }

    @Test
    fun `gridPhotoIds keeps the grid order in both id spaces`() {
        val mirror = listOf(
            photo(localId = "local-b", serverPhotoId = "srv-b"),
            photo(localId = "local-a", serverPhotoId = "srv-a"),
        )
        val grid = gridPhotoIds(listOf("srv-a", "srv-b"), mirror)

        assertEquals(listOf("srv-a", "srv-b"), grid.serverIds)
        assertEquals(listOf("local-a", "local-b"), grid.viewerIds)
    }

    // ── The two lists are legitimately different lengths ────────────────────

    @Test
    fun `an unmirrored photo keeps its tile but claims no page`() {
        // The grid draws thumbnails from the server, so it can render a photo
        // this device has never synced. The pager renders a PhotoEntity and
        // cannot. Carrying the id into the handoff anyway would promise a page
        // that does not exist.
        val mirror = listOf(photo(localId = "local-a", serverPhotoId = "srv-a"))
        val grid = gridPhotoIds(listOf("srv-a", "srv-unsynced"), mirror)

        assertEquals(2, grid.serverIds.size)
        assertEquals(listOf("local-a"), grid.viewerIds)
    }

    @Test
    fun `tapping an unmirrored tile falls back to the server id, which finds no page`() {
        // Deliberate: the fallback id is in nobody's mirror, so the viewer says
        // "Photo not found" with a log line. Do NOT "fix" this by coercing to
        // page 0 — that is the pre-E3 behaviour, and for a secured id it is a
        // confidentiality leak.
        val mirror = listOf(photo(localId = "local-a", serverPhotoId = "srv-a"))
        val grid = gridPhotoIds(listOf("srv-a", "srv-unsynced"), mirror)

        assertEquals("srv-unsynced", grid.viewerIdFor("srv-unsynced"))
        val resolved = resolvePhotos(
            orderPhotosBy(grid.viewerIds, mirror), emptySet(), null, collapseBurstStacks = false
        )
        assertEquals(-1, resolved.pageIndexOf(grid.viewerIdFor("srv-unsynced")))
    }

    @Test
    fun `orderPhotosBy drops an id with no row rather than yielding a hole`() {
        val mirror = listOf(photo(localId = "a"), photo(localId = "b"))
        assertEquals(
            listOf("a", "b"),
            orderPhotosBy(listOf("a", "gone", "b"), mirror).map { it.localId },
        )
    }

    @Test
    fun `orderPhotosBy de-duplicates because HorizontalPager is keyed on localId`() {
        // A repeated key crashes the pager. This is a crash guard, not tidiness.
        val mirror = listOf(photo(localId = "a"))
        val out = orderPhotosBy(listOf("a", "a", "a"), mirror)
        assertEquals(1, out.size)
        assertEquals(out.map { it.localId }.distinct().size, out.size)
    }

    // ── The secure filter still applies to a handed-over list ───────────────

    @Test
    fun `a secured photo is excluded from a handed-over list too`() {
        // The handoff comes from a SERVER endpoint — a face cluster or a trip has
        // no idea which of its photos the user later secured. If the pager took
        // the list as-is it would swipe straight into one, reopening exactly the
        // leak E3 closed, inside E3's own follow-up. resolveExplicit therefore
        // runs the list through the same excludeSecure as every other surface.
        val mirror = listOf(
            photo(localId = "a"),
            photo(localId = "secret"),
            photo(localId = "b"),
        )
        val handoff = listOf("a", "secret", "b")

        val resolved = resolvePhotos(
            members = orderPhotosBy(handoff, mirror),
            secureBlobIds = setOf("secret"),
            sort = null,
            collapseBurstStacks = false,
        )

        assertEquals(listOf("a", "b"), resolved.photos.map { it.localId })
        assertEquals(-1, resolved.pageIndexOf("secret"))
        // ...and every index after it shifts down, rather than the secured photo
        // silently occupying page 1.
        assertEquals(1, resolved.pageIndexOf("b"))
    }

    @Test
    fun `a handed-over list is never re-sorted or re-collapsed`() {
        // resolveExplicit passes sort = null, collapseBurstStacks = false: the
        // list IS the grid's resolved output, so applying either would be a
        // second derivation of one list — the defect E3 removed.
        val mirror = listOf(
            photo(localId = "old", takenAt = 100),
            photo(localId = "new", takenAt = 900),
        )
        val handoff = listOf("old", "new")

        val resolved = resolvePhotos(
            orderPhotosBy(handoff, mirror), emptySet(), null, collapseBurstStacks = false
        )
        assertEquals(handoff, resolved.photos.map { it.localId })
        assertNull(resolved.sort)
    }

    // ── Empty / degenerate inputs ───────────────────────────────────────────

    @Test
    fun `an empty grid produces an empty handoff, not a null one`() {
        // PhotoViewerViewModel treats an EMPTY handoff as absent and falls back
        // to the resolver, which is right: a grid that resolved nothing has
        // nothing to say about order, and the fallback at least renders.
        val grid = gridPhotoIds(emptyList(), emptyList())
        assertEquals(GridPhotoIds.EMPTY, grid)
        assertTrue(grid.viewerIds.isEmpty())
    }

    @Test
    fun `the handoff key has exactly one spelling`() {
        // NavGraph writes it, PhotoViewerViewModel reads it. A second spelling
        // would silently drop the handoff and fall back to the gallery order —
        // i.e. it would look precisely like the bug, with nothing failing.
        assertEquals("viewerPhotoIds", VIEWER_PHOTO_IDS_KEY)
    }
}
