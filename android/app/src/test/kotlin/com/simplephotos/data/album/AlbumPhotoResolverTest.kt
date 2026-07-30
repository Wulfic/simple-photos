package com.simplephotos.data.album

import com.simplephotos.data.collapseBursts
import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pins [resolvePhotos] — the single derivation the album grid and the viewer's
 * pager both render (E3, #52 follow-up).
 *
 * The bug was never in the comparator (`AlbumSortTest` already pins that); it
 * was that two surfaces built the list two different ways. So these tests assert
 * **equality of the two lists**, not "the pager is sorted": a pager that sorts
 * correctly but resolves a different *membership* still strands the user, and a
 * test that only checked order would pass while it did.
 *
 * [pagerListBeforeTheFix] reproduces what `PhotoViewerViewModel` used to do, so
 * every divergence below is asserted against the real defect rather than a
 * hypothetical one. Those cases fail if anyone reintroduces a second derivation.
 */
class AlbumPhotoResolverTest {

    private fun photo(
        localId: String,
        takenAt: Long = 0L,
        burstId: String? = null,
        serverBlobId: String? = localId,
        filename: String = "$localId.jpg",
    ) = PhotoEntity(
        localId = localId,
        filename = filename,
        takenAt = takenAt,
        mimeType = "image/jpeg",
        mediaType = "image",
        width = 100,
        height = 100,
        syncStatus = SyncStatus.SYNCED,
        createdAt = takenAt,
        isFavorite = false,
        serverBlobId = serverBlobId,
        burstId = burstId,
    )

    /**
     * What the pager did before this fix: `getAlbumPhotos` → `collapseBursts`,
     * with no secure exclusion and no sort. Kept here verbatim so the assertions
     * below prove the defect, not just the new behaviour.
     */
    private fun pagerListBeforeTheFix(members: List<PhotoEntity>): List<PhotoEntity> =
        members.collapseBursts()

    private fun ids(photos: List<PhotoEntity>) = photos.map { it.localId }

    // ── The reported symptom: the pager ignored the album's sort ─────────────

    @Test
    fun `grid and pager render the identical list under a name sort`() {
        val members = listOf(
            photo("c", takenAt = 300, filename = "IMG_10.jpg"),
            photo("a", takenAt = 100, filename = "IMG_2.jpg"),
            photo("b", takenAt = 200, filename = "IMG_1.jpg"),
        )
        val sort = AlbumSort(AlbumSortField.NAME, AlbumSortDir.ASC)

        val resolved = resolvePhotos(members, emptySet(), sort, collapseBurstStacks = false)

        // Natural order: IMG_1 < IMG_2 < IMG_10.
        assertEquals(listOf("b", "a", "c"), ids(resolved.photos))
        // The grid renders `photos` and the pager pages `photos` — one field, so
        // the only thing left to prove is that it is genuinely different from
        // the list the pager used to build for itself.
        assertNotEquals(ids(resolved.photos), ids(pagerListBeforeTheFix(members)))
    }

    @Test
    fun `the first frame was right and every swipe after it was wrong`() {
        // The reported shape: the tapped photo is located by id, so page 0 always
        // matched. Page 1 was whatever the unsorted query happened to return.
        val members = listOf(
            photo("old", takenAt = 100),
            photo("new", takenAt = 300),
            photo("mid", takenAt = 200),
        )
        val sort = AlbumSort(AlbumSortField.DATE, AlbumSortDir.ASC)

        val grid = resolvePhotos(members, emptySet(), sort, collapseBurstStacks = false).photos
        val oldPager = pagerListBeforeTheFix(members)

        val tapped = grid.indexOfFirst { it.localId == "old" }
        assertEquals(0, tapped)
        assertEquals(0, oldPager.indexOfFirst { it.localId == "old" })
        // Same first page, different second page — exactly the report.
        assertEquals("mid", grid[tapped + 1].localId)
        assertEquals("new", oldPager[tapped + 1].localId)
    }

    @Test
    fun `no sort keeps the intrinsic order untouched`() {
        // "Recently Added" depends on this: a user who never chose a sort must
        // see the pre-#52 add-order, not a default date sort.
        val members = listOf(photo("z", takenAt = 100), photo("a", takenAt = 300))
        val resolved = resolvePhotos(members, emptySet(), sort = null, collapseBurstStacks = false)
        assertEquals(listOf("z", "a"), ids(resolved.photos))
    }

    // ── Defect 1: the pager paged into secured photos ────────────────────────

    @Test
    fun `a secured member is absent from the pager, not merely reordered`() {
        val members = listOf(photo("a"), photo("secret"), photo("b"))

        val resolved = resolvePhotos(members, setOf("secret"), null, collapseBurstStacks = false)

        assertEquals(listOf("a", "b"), ids(resolved.photos))
        // The confidentiality bug: the old pager swiped straight into it.
        assertTrue(pagerListBeforeTheFix(members).any { it.localId == "secret" })
    }

    @Test
    fun `securing a photo shifts no index because it never occupies one`() {
        val members = listOf(photo("a"), photo("secret"), photo("b"), photo("c"))
        val resolved = resolvePhotos(members, setOf("secret"), null, collapseBurstStacks = false)
        // "b" is at index 1 for the grid AND the pager. Before the fix the grid
        // said 1 and the pager said 2, so every tap past a secured photo opened
        // the neighbour.
        assertEquals(1, resolved.photos.indexOfFirst { it.localId == "b" })
        assertEquals(2, pagerListBeforeTheFix(members).indexOfFirst { it.localId == "b" })
    }

    @Test
    fun `a secured frame can never become a burst representative`() {
        // Exclusion runs BEFORE collapse, so the visible cover is the first
        // NON-secured frame — not a hidden one the grid would refuse to draw.
        val members = listOf(
            photo("secret", burstId = "burst1"),
            photo("frame2", burstId = "burst1"),
            photo("frame3", burstId = "burst1"),
        )
        val resolved = resolvePhotos(members, setOf("secret"), null, collapseBurstStacks = true)
        assertEquals(listOf("frame2"), ids(resolved.photos))
    }

    // ── Defect 2: the burst policy differed between grid and pager ───────────

    @Test
    fun `a regular album keeps every burst frame, matching web and the grid`() {
        // web/src/hooks/useAlbumPhotos.ts collapses for SMART albums only:
        // regular albums keep every frame the user explicitly added so removal
        // stays faithful to the manifest. The old pager collapsed regardless.
        val members = listOf(
            photo("f1", burstId = "burst1"),
            photo("f2", burstId = "burst1"),
            photo("solo"),
        )
        val resolved = resolvePhotos(members, emptySet(), null, collapseBurstStacks = false)

        assertEquals(listOf("f1", "f2", "solo"), ids(resolved.photos))
        assertEquals(listOf("f1", "solo"), ids(pagerListBeforeTheFix(members)))
    }

    @Test
    fun `the gallery collapses bursts so a stack is one swipe`() {
        val members = listOf(
            photo("f1", burstId = "burst1"),
            photo("f2", burstId = "burst1"),
            photo("solo"),
        )
        val resolved = resolvePhotos(members, emptySet(), null, collapseBurstStacks = true)

        assertEquals(listOf("f1", "solo"), ids(resolved.photos))
        // members stays un-collapsed so the filmstrip can still resolve frames.
        assertEquals(listOf("f1", "f2", "solo"), ids(resolved.members))
    }

    @Test
    fun `collapse runs before the sort so a sort change never swaps the cover`() {
        // f2 is newer than f1. Collapse-then-sort keeps f1 (the intrinsic cover,
        // the image the gallery shows) as the representative under either
        // direction; sort-then-collapse would swap the tile's image on a tap.
        val members = listOf(
            photo("f1", takenAt = 100, burstId = "burst1"),
            photo("f2", takenAt = 900, burstId = "burst1"),
            photo("solo", takenAt = 500),
        )
        val asc = resolvePhotos(
            members, emptySet(), AlbumSort(AlbumSortField.DATE, AlbumSortDir.ASC), true
        )
        val desc = resolvePhotos(
            members, emptySet(), AlbumSort(AlbumSortField.DATE, AlbumSortDir.DESC), true
        )
        assertEquals(listOf("f1", "solo"), ids(asc.photos))
        assertEquals(listOf("solo", "f1"), ids(desc.photos))
    }

    // ── The invariant the grid's re-sort depends on ──────────────────────────

    @Test
    fun `photos always equals tiles run through the sort`() {
        // AlbumDetailViewModel re-orders on a sort tap by calling
        // sortAlbumPhotos(tiles, sort) without reloading. If that ever stopped
        // reproducing resolvePhotos' own output, the grid would drift from the
        // pager again between a sort tap and the next load.
        val members = listOf(
            photo("a", takenAt = 300, burstId = "burst1"),
            photo("b", takenAt = 100, burstId = "burst1"),
            photo("secret", takenAt = 200),
            photo("c", takenAt = 400),
        )
        for (sort in listOf(
            null,
            AlbumSort(AlbumSortField.DATE, AlbumSortDir.ASC),
            AlbumSort(AlbumSortField.DATE, AlbumSortDir.DESC),
            AlbumSort(AlbumSortField.NAME, AlbumSortDir.ASC),
        )) {
            for (collapse in listOf(true, false)) {
                val r = resolvePhotos(members, setOf("secret"), sort, collapse)
                assertEquals(
                    "sort=$sort collapse=$collapse",
                    r.photos,
                    sortAlbumPhotos(r.tiles, r.sort),
                )
            }
        }
    }

    // ── Which page a tap opens ──────────────────────────────────────────────

    @Test
    fun `a tapped photo opens on its own page`() {
        val members = listOf(photo("a", takenAt = 100), photo("b", takenAt = 300))
        val r = resolvePhotos(
            members, emptySet(), AlbumSort(AlbumSortField.DATE, AlbumSortDir.DESC), false
        )
        // Sorted to [b, a] — the tap must follow the sort, not the query order.
        assertEquals(0, r.pageIndexOf("b"))
        assertEquals(1, r.pageIndexOf("a"))
    }

    @Test
    fun `a collapsed-away burst frame opens on its stack's cover`() {
        val members = listOf(
            photo("f1", burstId = "burst1"),
            photo("f2", burstId = "burst1"),
            photo("solo"),
        )
        val r = resolvePhotos(members, emptySet(), null, collapseBurstStacks = true)
        // f2 has no page of its own; its stack's cover does.
        assertEquals(0, r.pageIndexOf("f2"))
    }

    @Test
    fun `a secured photo reports no page rather than opening a stranger's`() {
        // Search and the people/pets/memories/trips grids resolve from server
        // endpoints, so they can hand the viewer a secured id. Answering 0 would
        // open an unrelated photo — and would reopen the leak for a secured one.
        val members = listOf(photo("a"), photo("secret"), photo("b"))
        val r = resolvePhotos(members, setOf("secret"), null, collapseBurstStacks = false)
        assertEquals(-1, r.pageIndexOf("secret"))
    }

    @Test
    fun `an id absent from the mirror reports no page`() {
        val r = resolvePhotos(listOf(photo("a")), emptySet(), null, collapseBurstStacks = false)
        assertEquals(-1, r.pageIndexOf("never-synced"))
    }

    // ── The persisted key both surfaces read ────────────────────────────────

    @Test
    fun `both surfaces read one spelling of the sort preference key`() {
        // The grid writes this key and the pager reads it. A second spelling
        // would silently hand the pager a different order — the original bug in
        // a new disguise.
        assertEquals("albumSort:my-album", albumSortPrefKey("my-album").name)
    }
}
