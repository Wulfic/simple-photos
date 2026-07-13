package com.simplephotos.data

import com.simplephotos.data.local.entities.PhotoEntity
import com.simplephotos.data.local.entities.SyncStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test

/**
 * Pins the shared [excludeSecure] filter (com.simplephotos.data) used by the
 * main gallery, album-detail grids, and album/smart counts so that photos moved
 * into a secure gallery are hidden EVERYWHERE, not just the main grid (#16 —
 * "secure albums don't fully remove media from the regular gallery").
 */
class SecureExclusionTest {

    private fun photo(
        localId: String,
        serverBlobId: String? = localId,
    ) = PhotoEntity(
        localId = localId,
        filename = "$localId.jpg",
        takenAt = 0L,
        mimeType = "image/jpeg",
        mediaType = "image",
        width = 100,
        height = 100,
        syncStatus = SyncStatus.SYNCED,
        createdAt = 0L,
        isFavorite = false,
        serverBlobId = serverBlobId,
    )

    @Test
    fun `removes every photo whose server blob id is secured`() {
        val src = listOf(photo("a"), photo("secret"), photo("b"), photo("secret2"))
        val out = src.excludeSecure(setOf("secret", "secret2"))
        assertEquals(listOf("a", "b"), out.map { it.localId })
    }

    @Test
    fun `local-only photos (no server blob id) always pass through`() {
        val src = listOf(photo("local", serverBlobId = null), photo("secret"))
        val out = src.excludeSecure(setOf("secret"))
        assertEquals(listOf("local"), out.map { it.localId })
    }

    @Test
    fun `empty secure set returns the receiver unchanged (identity, no copy)`() {
        val src = listOf(photo("a"), photo("b"))
        val out = src.excludeSecure(emptySet())
        assertSame(src, out)
    }

    @Test
    fun `count of excluded list is what the album badge must report`() {
        // The regular-album badge is excludeSecure(members).size — this is the
        // invariant that keeps the count equal to the rendered grid.
        val members = listOf(photo("a"), photo("secret"), photo("b"))
        assertEquals(2, members.excludeSecure(setOf("secret")).size)
    }
}
