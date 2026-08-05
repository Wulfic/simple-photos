package com.simplephotos.data

import com.simplephotos.data.remote.dto.SecureGalleryItem
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pins the pure [SecureSmartAlbums] classifier that drives the built-in secure
 * smart albums (Secure Gallery / Photos / GIFs / Videos / Audio). Mirrors the
 * web `secureSmartAlbums.test.ts`. Covers NULL media_type, per-type filters,
 * count>0 visibility, and newest-first cover selection.
 */
class SecureSmartAlbumsTest {

    private fun item(
        id: String,
        mediaType: String?,
        galleryId: String = "g1",
    ) = SecureGalleryItem(
        id = id,
        blobId = "blob-$id",
        addedAt = "2026-07-16T00:00:00Z",
        galleryId = galleryId,
        mediaType = mediaType,
    )

    @Test
    fun `isSmart only matches the secure-smart namespace`() {
        assertTrue(SecureSmartAlbums.isSmart("secure-smart-all"))
        assertTrue(SecureSmartAlbums.isSmart("secure-smart-videos"))
        assertFalse(SecureSmartAlbums.isSmart("smart-photos")) // main-gallery ns
        assertFalse(SecureSmartAlbums.isSmart("some-uuid"))
        assertFalse(SecureSmartAlbums.isSmart(null))
    }

    @Test
    fun `labelOf resolves ids`() {
        assertEquals("GIFs", SecureSmartAlbums.labelOf("secure-smart-gifs"))
        assertNull(SecureSmartAlbums.labelOf("nope"))
    }

    @Test
    fun `photos filter includes photo, gif and NULL media_type`() {
        val f = { mt: String? -> SecureSmartAlbums.filter(listOf(item("x", mt)), SecureSmartAlbums.PHOTOS).size }
        assertEquals(1, f("photo"))
        assertEquals(1, f("gif"))
        assertEquals(1, f(null))
        assertEquals(0, f("video"))
        assertEquals(0, f("audio"))
    }

    @Test
    fun `gifs videos audio are exact matches, excluding NULL`() {
        assertEquals(1, SecureSmartAlbums.filter(listOf(item("a", "gif")), SecureSmartAlbums.GIFS).size)
        assertEquals(0, SecureSmartAlbums.filter(listOf(item("a", null)), SecureSmartAlbums.GIFS).size)
        assertEquals(1, SecureSmartAlbums.filter(listOf(item("a", "video")), SecureSmartAlbums.VIDEOS).size)
        assertEquals(0, SecureSmartAlbums.filter(listOf(item("a", null)), SecureSmartAlbums.VIDEOS).size)
        assertEquals(1, SecureSmartAlbums.filter(listOf(item("a", "audio")), SecureSmartAlbums.AUDIO).size)
    }

    @Test
    fun `compute returns only non-empty albums with correct counts`() {
        val items = listOf(
            item("v1", "video"),
            item("p1", "photo"),
            item("g1", "gif"),
            item("p2", "photo"),
        )
        val byId = SecureSmartAlbums.compute(items).associateBy { it.id }
        assertFalse(byId.containsKey(SecureSmartAlbums.AUDIO)) // no audio → no tile
        assertEquals(4, byId[SecureSmartAlbums.ALL]!!.count)
        assertEquals(3, byId[SecureSmartAlbums.PHOTOS]!!.count) // 2 photos + 1 gif
        assertEquals(1, byId[SecureSmartAlbums.GIFS]!!.count)
        assertEquals(1, byId[SecureSmartAlbums.VIDEOS]!!.count)
    }

    @Test
    fun `NULL media_type lands in Photos and Secure Gallery only`() {
        val ids = SecureSmartAlbums.compute(listOf(item("x", null))).map { it.id }
        assertTrue(ids.contains(SecureSmartAlbums.ALL))
        assertTrue(ids.contains(SecureSmartAlbums.PHOTOS))
        assertFalse(ids.contains(SecureSmartAlbums.GIFS))
        assertFalse(ids.contains(SecureSmartAlbums.VIDEOS))
        assertFalse(ids.contains(SecureSmartAlbums.AUDIO))
    }

    @Test
    fun `cover is the first (newest) matching item in the input order`() {
        // Input is added_at DESC (newest first) per the server contract.
        val items = listOf(
            item("newest-video", "video"),
            item("old-photo", "photo"),
            item("newer-photo", "photo"),
        )
        val byId = SecureSmartAlbums.compute(items).associateBy { it.id }
        assertEquals("newest-video", byId[SecureSmartAlbums.ALL]!!.coverItem.id)
        assertEquals("old-photo", byId[SecureSmartAlbums.PHOTOS]!!.coverItem.id)
        assertEquals("newest-video", byId[SecureSmartAlbums.VIDEOS]!!.coverItem.id)
    }

    @Test
    fun `empty input yields no albums`() {
        assertTrue(SecureSmartAlbums.compute(emptyList()).isEmpty())
    }

    @Test
    fun `filter with unknown id returns empty`() {
        assertTrue(SecureSmartAlbums.filter(listOf(item("a", "photo")), "not-smart").isEmpty())
    }

    @Test
    fun `defs are ordered all, photos, gifs, videos, audio`() {
        val allTypes = listOf(
            item("a", "photo"), item("b", "gif"),
            item("c", "video"), item("d", "audio"),
        )
        assertEquals(
            listOf(
                SecureSmartAlbums.ALL,
                SecureSmartAlbums.PHOTOS,
                SecureSmartAlbums.GIFS,
                SecureSmartAlbums.VIDEOS,
                SecureSmartAlbums.AUDIO,
            ),
            SecureSmartAlbums.compute(allTypes).map { it.id },
        )
    }
}
