package com.simplephotos.data.sync

import com.simplephotos.data.local.entities.PhotoEntity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The #38 delta-sync decisions, tested as arithmetic.
 *
 * Everything here is a case where getting it wrong costs rows permanently
 * rather than visibly, which is exactly the class of bug a device test does not
 * catch — the mirror looks fine until the day it is short and nothing explains
 * why.
 */
class SyncPlanTest {

    // ── decideSyncMode ─────────────────────────────────────────────────────

    @Test
    fun `no cursor forces a full walk`() {
        assertEquals(SyncMode.FULL, decideSyncMode(cursor = null, headSeq = 400L))
    }

    @Test
    fun `matching head skips the pass entirely`() {
        assertEquals(SyncMode.SKIPPED, decideSyncMode(cursor = 400L, headSeq = 400L))
    }

    @Test
    fun `advanced head takes the delta`() {
        assertEquals(SyncMode.DELTA, decideSyncMode(cursor = 400L, headSeq = 412L))
    }

    /**
     * Losing the summary call costs the shortcut, not correctness — the delta
     * response carries its own head, and the `deleted` handshake still catches a
     * server too old to honour `since`. Forcing a full walk here would mean one
     * flaky summary request re-paginating a 15k-row library.
     */
    @Test
    fun `unknown head still takes the delta rather than a full walk`() {
        assertEquals(SyncMode.DELTA, decideSyncMode(cursor = 400L, headSeq = null))
    }

    /**
     * A head BELOW our cursor cannot happen on a server that has merely been
     * running — the log is global and monotonic. It means a restored database or
     * a different server, so our cursor indexes a sequence space that no longer
     * exists. A delta would report "nothing above 400" forever while the mirror
     * holds rows this server has never heard of.
     */
    @Test
    fun `rewound head forces a full walk`() {
        assertEquals(SyncMode.FULL, decideSyncMode(cursor = 400L, headSeq = 12L))
    }

    @Test
    fun `a zero cursor is a real cursor, not an absent one`() {
        // seq 0 means "current as of an empty log" — a legitimate state on a
        // fresh library, and distinct from null. Treating it as absent would
        // force a pointless full walk on every pass for such an account.
        assertEquals(SyncMode.SKIPPED, decideSyncMode(cursor = 0L, headSeq = 0L))
        assertEquals(SyncMode.DELTA, decideSyncMode(cursor = 0L, headSeq = 5L))
    }

    @Test
    fun `a negative cursor is corrupt and forces a full walk`() {
        assertEquals(SyncMode.FULL, decideSyncMode(cursor = -1L, headSeq = 400L))
    }

    // ── isDeltaFeed ────────────────────────────────────────────────────────

    /**
     * The handshake. A server predating #38 ignores `since` and replies with a
     * full walk whose `photos` are indistinguishable from a delta's; the ONLY
     * thing separating them on the wire is whether `deleted` is present.
     *
     * Getting this backwards is silent and permanent: the client prunes nothing
     * while believing it pruned, then persists a cursor that makes it so.
     */
    @Test
    fun `an empty deleted array is a delta, an absent one is not`() {
        assertTrue("empty means 'a delta with no departures'", isDeltaFeed(emptyList()))
        assertTrue(isDeltaFeed(listOf("p1")))
        assertFalse("absent means the server ignored `since`", isDeltaFeed(null))
    }

    // ── tombstoneVictims ───────────────────────────────────────────────────

    private fun row(
        localId: String,
        serverPhotoId: String? = null,
        localPath: String? = null,
    ) = PhotoEntity(
        localId = localId,
        serverPhotoId = serverPhotoId,
        filename = "$localId.jpg",
        takenAt = 0L,
        mimeType = "image/jpeg",
        width = 100,
        height = 100,
        localPath = localPath,
    )

    @Test
    fun `a tombstoned server row is removed`() {
        val rows = listOf(row("a", serverPhotoId = "p1"), row("b", serverPhotoId = "p2"))
        val victims = tombstoneVictims(rows, setOf("p1"))
        assertEquals(listOf("a"), victims.map { it.localId })
    }

    /**
     * The guard that keeps the delta path in agreement with the full walk.
     *
     * `reconcileServerDeletions` never removes a row with a `localPath`, so a
     * photo captured on this device, uploaded, then merged survives a
     * server-side deletion. If the delta deleted it anyway, the next cold start
     * could not restore it — the server no longer lists the photo, so the walk
     * that is supposed to be self-healing has nothing to heal from, and a photo
     * still physically on the phone is gone from the library for good.
     */
    @Test
    fun `a device-captured row survives its tombstone, matching the full walk`() {
        val rows = listOf(row("a", serverPhotoId = "p1", localPath = "content://media/1"))
        assertEquals(emptyList<String>(), tombstoneVictims(rows, setOf("p1")).map { it.localId })
    }

    @Test
    fun `a row that never came from the server is untouched`() {
        // No serverPhotoId at all: a pending upload. A tombstone cannot name it,
        // but a bug that matched on localId instead would delete it.
        val rows = listOf(row("p1", serverPhotoId = null, localPath = "content://media/1"))
        assertEquals(emptyList<String>(), tombstoneVictims(rows, setOf("p1")).map { it.localId })
    }

    @Test
    fun `no tombstones removes nothing`() {
        val rows = listOf(row("a", serverPhotoId = "p1"))
        assertEquals(emptyList<String>(), tombstoneVictims(rows, emptySet()).map { it.localId })
    }

    @Test
    fun `a tombstone for a photo this device never held is a no-op`() {
        val rows = listOf(row("a", serverPhotoId = "p1"))
        assertEquals(emptyList<String>(), tombstoneVictims(rows, setOf("p999")).map { it.localId })
    }
}
