package com.simplephotos.data.repository

import com.simplephotos.data.repository.AlbumRepository.Companion.resolveAlbumDisplayName
import com.simplephotos.data.repository.AlbumRepository.Companion.sourceAlbumId
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

/**
 * Parity tests for the Takeout album display-name rule.
 *
 * These deliberately mirror `web/src/utils/takeoutAlbums.test.ts` case for case:
 * both platforms rebuild the same albums from the same server mapping, so if the
 * two rules disagree the devices rename each other's albums back and forth on
 * every sync.
 */
class AlbumDisplayNameTest {
    // The mangling Google actually applies on export: "&" and "'" collapse to "_".
    private val folder = "Mum _ Dad_s 40th"
    private val title = "Mum & Dad's 40th"

    @Test
    fun `shows the real title instead of the mangled folder name`() {
        assertEquals(title, resolveAlbumDisplayName(folder, title, null))
    }

    @Test
    fun `falls back to the folder name when the export carried no title`() {
        assertEquals("Trip to Rome", resolveAlbumDisplayName("Trip to Rome", null, null))
        assertEquals("Trip to Rome", resolveAlbumDisplayName("Trip to Rome", "   ", null))
    }

    @Test
    fun `re-titles an album still carrying the raw folder name`() {
        assertEquals(title, resolveAlbumDisplayName(folder, title, folder))
    }

    @Test
    fun `leaves a user's own rename alone`() {
        assertEquals(
            "The 40th Party",
            resolveAlbumDisplayName(folder, title, "The 40th Party"),
        )
    }

    @Test
    fun `is stable once applied, so a re-run is a no-op`() {
        val first = resolveAlbumDisplayName(folder, title, folder)
        assertEquals(first, resolveAlbumDisplayName(folder, title, first))
    }

    @Test
    fun `does not rename when there is no title to rename to`() {
        assertEquals(
            "Trip to Rome",
            resolveAlbumDisplayName("Trip to Rome", null, "Trip to Rome"),
        )
    }

    @Test
    fun `source album id matches the shared cross-platform formula`() {
        // The SAME vector is pinned in the server
        // (`source_album_id_matches_the_client_formula`) and web
        // (`takeoutAlbums.test.ts`). Three codebases compute this id
        // independently and every drift is silent: albums duplicate instead of
        // converging, and a delete tombstone stops matching so the album returns.
        // Reference: `printf 'google_takeout Trip to Rome' | sha256sum`.
        assertEquals(
            "src-03c6bc29608fa7bffdbdd7b46dab34de74aa131875c032e79ab581a44a29e672",
            sourceAlbumId("google_takeout", "Trip to Rome"),
        )
    }

    @Test
    fun `source album id keys on the folder name, not the title`() {
        // Identity must not move when an album is retitled — otherwise every
        // device orphans its existing album and builds a duplicate.
        assertNotEquals(
            sourceAlbumId("google_takeout", folder),
            sourceAlbumId("google_takeout", title),
        )
    }
}
