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
        serverPhotoId: String? = null,
    ) = PhotoEntity(
        serverPhotoId = serverPhotoId,
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
    fun `a row secured by its PHOTO id is hidden even when its blob id is its own upload`() {
        // TODO.md S1, measured 2026-11-17: the phone re-uploaded 158 photos that
        // were in secure albums. register-encrypted mapped each onto the hidden
        // server photo, but the row kept the blob IT uploaded — not in the set —
        // so it showed in the main grid. The server hides by
        // `photos.id NOT IN (original_blob_id)`, and `/galleries/secure/blob-ids`
        // already carries those photo ids (158/158 verified live).
        val reuploaded = photo("dev", serverBlobId = "blob-the-phone-uploaded", serverPhotoId = "secured-photo")
        val ordinary = photo("ok", serverBlobId = "b-ok", serverPhotoId = "p-ok")
        val out = listOf(reuploaded, ordinary).excludeSecure(setOf("secured-photo", "some-clone-blob"))
        assertEquals(listOf("ok"), out.map { it.localId })
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
