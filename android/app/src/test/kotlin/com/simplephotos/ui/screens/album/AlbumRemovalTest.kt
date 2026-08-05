package com.simplephotos.ui.screens.album

import com.simplephotos.data.remote.dto.SecureGalleryRef
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Mirrors `web/src/gallery/albumRemoval.test.ts`.
 *
 * The wording is the only part of a confirmation dialog a JVM test can reach —
 * Compose UI needs a device — and it is also the part that can be *wrong*. Z1e
 * exists because the Android dialog promised "will return to your regular
 * gallery" unconditionally, long after that stopped being true.
 */
class AlbumRemovalTest {

    private fun ref(id: String) = SecureGalleryRef(id = id, name = id.uppercase())

    // ── otherSecureAlbumCount ───────────────────────────────────────────────

    @Test
    fun `a photo in only the owning album has no other albums`() {
        assertEquals(0, AlbumRemoval.otherSecureAlbumCount(listOf(ref("g1")), "g1"))
    }

    @Test
    fun `counts every membership except the owning one`() {
        assertEquals(
            2,
            AlbumRemoval.otherSecureAlbumCount(listOf(ref("g1"), ref("g2"), ref("g3")), "g1"),
        )
    }

    @Test
    fun `an empty list is UNKNOWN, not zero`() {
        // The whole point. The server documents a miss as unreachable by
        // construction, so empty can only mean the feed published nothing —
        // and reading it as 0 is how the UI came to promise the wrong outcome.
        assertEquals(null, AlbumRemoval.otherSecureAlbumCount(emptyList(), "g1"))
    }

    @Test
    fun `a null list is UNKNOWN`() {
        // Realistic on Android: Gson leaves an absent or renamed wire key at its
        // default, so a server change or a typo lands here rather than crashing.
        assertEquals(null, AlbumRemoval.otherSecureAlbumCount(null, "g1"))
    }

    @Test
    fun `a list missing its own owning album is UNKNOWN, not off by one`() {
        // Counting the owner as an "other" would flip a last-membership removal
        // into the "stays secured" branch — wrong in the surprising direction.
        assertEquals(null, AlbumRemoval.otherSecureAlbumCount(listOf(ref("g2")), "g1"))
    }

    // ── albumRemovalPrompt (ordinary albums) ────────────────────────────────

    @Test
    fun `an ordinary removal says nothing is deleted`() {
        val p = AlbumRemoval.albumRemovalPrompt(3, "Holiday")
        assertEquals("Remove 3 photos from “Holiday”?", p.title)
        assertTrue(p.body, p.body.contains("Nothing is deleted"))
        assertTrue(p.body, p.body.contains("They stay in your gallery"))
    }

    @Test
    fun `a single ordinary removal is singular`() {
        val p = AlbumRemoval.albumRemovalPrompt(1, "Holiday")
        assertEquals("Remove 1 photo from “Holiday”?", p.title)
        assertTrue(p.body, p.body.contains("It stays"))
    }

    @Test
    fun `an unnamed album is described rather than quoted empty`() {
        val p = AlbumRemoval.albumRemovalPrompt(2, null)
        assertEquals("Remove 2 photos from this album?", p.title)
    }

    // ── secureRemovalPrompt ─────────────────────────────────────────────────

    @Test
    fun `a last membership promises the photo comes back`() {
        val v = AlbumRemoval.secureRemovalPrompt(listOf(0), "Private")
        assertTrue(v is SecureRemovalVerdict.Confirm)
        assertTrue(v.prompt.body, v.prompt.body.contains("visible in your regular gallery again"))
    }

    @Test
    fun `another secure album means it does NOT come back`() {
        val v = AlbumRemoval.secureRemovalPrompt(listOf(1), "Private")
        assertTrue(v is SecureRemovalVerdict.Confirm)
        // The exact claim Z1 made false. If this body ever contains the
        // unconditional promise again, this is the test that says so.
        assertTrue(v.prompt.body, v.prompt.body.contains("will stay secured"))
        assertTrue(v.prompt.body, v.prompt.body.contains("NOT return to your regular gallery"))
        assertTrue(v.prompt.body, v.prompt.body.contains("1 other secure album"))
    }

    @Test
    fun `two other albums pluralise`() {
        val v = AlbumRemoval.secureRemovalPrompt(listOf(2), "Private")
        assertTrue(v.prompt.body, v.prompt.body.contains("2 other secure albums"))
    }

    @Test
    fun `unknown membership BLOCKS rather than guessing`() {
        val v = AlbumRemoval.secureRemovalPrompt(listOf(null), "Private")
        assertTrue(v is SecureRemovalVerdict.Blocked)
        assertTrue(v.prompt.title, v.prompt.title.startsWith("Can't remove"))
        assertTrue(v.prompt.body, v.prompt.body.contains("Refresh"))
    }

    @Test
    fun `one unknown item in a batch blocks the whole batch`() {
        // Fail closed: a batch is only as knowable as its least-known member,
        // and the prompt speaks for all of them at once.
        val v = AlbumRemoval.secureRemovalPrompt(listOf(0, 0, null), "Private")
        assertTrue(v is SecureRemovalVerdict.Blocked)
    }

    @Test
    fun `an empty batch is UNKNOWN, not a promise`() {
        // A caller that resolved nothing has not answered the question. The safe
        // reading of "no information" is never "no other album".
        val v = AlbumRemoval.secureRemovalPrompt(emptyList(), "Private")
        assertTrue(v is SecureRemovalVerdict.Blocked)
    }

    @Test
    fun `a batch where every item is last says they all come back`() {
        val v = AlbumRemoval.secureRemovalPrompt(listOf(0, 0, 0), "Private")
        assertEquals("Remove 3 photos from “Private”?", v.prompt.title)
        assertTrue(v.prompt.body, v.prompt.body.contains("They will be unsecured"))
    }

    @Test
    fun `a batch where every item is shared claims no count it cannot support`() {
        // Per-item counts differ, so the batch says "each is also in another
        // secure album" rather than averaging them into a number nobody asked for.
        val v = AlbumRemoval.secureRemovalPrompt(listOf(1, 3), "Private")
        assertTrue(v.prompt.body, v.prompt.body.contains("each is also in another secure album"))
        assertTrue(v.prompt.body, v.prompt.body.contains("NOT return"))
    }

    @Test
    fun `a mixed batch states both halves with their counts`() {
        // The one case no single sentence can state honestly: two of these leave
        // the secure domain and one does not.
        val v = AlbumRemoval.secureRemovalPrompt(listOf(0, 0, 2), "Private")
        assertTrue(v is SecureRemovalVerdict.Confirm)
        assertTrue(v.prompt.body, v.prompt.body.contains("2 will return to your regular gallery"))
        assertTrue(v.prompt.body, v.prompt.body.contains("The other 1 will stay secured"))
    }

    @Test
    fun `a secure prompt with no album name never renders empty quotes`() {
        val v = AlbumRemoval.secureRemovalPrompt(listOf(0), null)
        assertEquals("Remove 1 photo from this secure album?", v.prompt.title)
    }
}
