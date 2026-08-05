package com.simplephotos.data.repository

import com.simplephotos.data.repository.AlbumRepository.Companion.takeoutSettled
import com.simplephotos.data.repository.AlbumRepository.Companion.visibleMemberCount
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The album tile badge's rule. Mirrors web's `countRegularAlbum` tests case for
 * case: the two platforms count the same album from the same manifest, so a
 * disagreement here is a user-visible "it says 40 on my phone and 38 on the
 * laptop".
 */
class VisibleMemberCountTest {
    private val mirror = setOf("b1", "b2", "b3", "b4")

    @Test
    fun `counts members present in the mirror`() {
        assertEquals(3, visibleMemberCount(listOf("b1", "b2", "b3"), mirror, emptySet()))
    }

    @Test
    fun `ignores members the mirror has never seen`() {
        // Not yet synced, or deleted server-side: either way the grid can't show
        // it, so the badge must not count it (#12).
        assertEquals(2, visibleMemberCount(listOf("b1", "b2", "not-synced"), mirror, emptySet()))
    }

    @Test
    fun `excludes members inside a secure gallery`() {
        // Secured photos are hidden from every grid, so counting them makes the
        // badge over-report against the album it opens (#16).
        assertEquals(2, visibleMemberCount(listOf("b1", "b2", "b3"), mirror, setOf("b3")))
    }

    @Test
    fun `a secure member missing from the mirror is not double-excluded`() {
        assertEquals(1, visibleMemberCount(listOf("b1", "gone"), mirror, setOf("gone")))
    }

    @Test
    fun `an empty album counts zero`() {
        assertEquals(0, visibleMemberCount(emptyList(), mirror, emptySet()))
    }

    @Test
    fun `is stable when nothing changed`() {
        // The actual reported bug: the same inputs must always produce the same
        // number, however many times a resume recomputes it.
        val ids = listOf("b1", "b2", "b3", "unsynced")
        val first = visibleMemberCount(ids, mirror, setOf("b2"))
        repeat(5) {
            assertEquals(first, visibleMemberCount(ids, mirror, setOf("b2")))
        }
    }
}

/** The rule that decides when Takeout reconstruction can stop re-running. */
class TakeoutSettledTest {
    private fun result(
        created: Int = 0,
        updated: Int = 0,
        added: Int = 0,
        unmatched: Int = 0,
    ) = AlbumRepository.SourceAlbumRebuildResult(
        albumsCreated = created,
        albumsUpdated = updated,
        photosAdded = added,
        photosUnmatched = unmatched,
    )

    @Test
    fun `settles when every source photo matched`() {
        assertTrue(takeoutSettled(result(created = 3, added = 40, unmatched = 0), -1))
    }

    @Test
    fun `keeps running while photos are still arriving`() {
        assertFalse(takeoutSettled(result(updated = 1, added = 5, unmatched = 12), 20))
    }

    @Test
    fun `keeps running when the gap shrank, even on a no-op pass`() {
        // The gap is still closing — more photos are syncing in.
        assertFalse(takeoutSettled(result(unmatched = 12), 20))
    }

    @Test
    fun `settles on an unchanged pass that left an identical gap`() {
        // Photos that were trashed or secured never sync, so `unmatched` can
        // never reach 0. Without this, the pass re-runs forever.
        assertTrue(takeoutSettled(result(unmatched = 12), 12))
    }

    @Test
    fun `does not settle on the first pass just because a gap exists`() {
        // previousUnmatched = -1 is "no previous pass" — never let that match.
        assertFalse(takeoutSettled(result(unmatched = 12), -1))
    }

    @Test
    fun `does not settle when the gap is identical but work was done`() {
        // Photos were added while the same number stayed unmatched: the mirror is
        // still moving, so give the next pass a chance.
        assertFalse(takeoutSettled(result(added = 3, unmatched = 12), 12))
    }
}
