package com.simplephotos.data.repository

import com.simplephotos.data.local.entities.AlbumEntity
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The album manifest is end-to-end encrypted, so the server can never validate
 * it — these tests and the matching web ones are the only thing holding the two
 * clients to the same format.
 */
class AlbumManifestTest {
    private fun album(
        id: String = "album-1",
        name: String = "Trip to Rome",
        photoBlobIds: List<String> = emptyList(),
    ) = AlbumEntity(
        localId = id,
        name = name,
        createdAt = 1_700_000_000_000L,
        photoBlobIds = photoBlobIds,
    )

    @Test
    fun `round-trips an album's membership`() {
        val ids = listOf("blob-a", "blob-b", "blob-c")
        val parsed = AlbumManifest.parse(AlbumManifest.payloadFor(album(photoBlobIds = ids), "blob-a"))

        assertEquals("album-1", parsed.albumId)
        assertEquals("Trip to Rome", parsed.name)
        assertEquals(1_700_000_000_000L, parsed.createdAt)
        assertEquals("blob-a", parsed.coverPhotoBlobId)
        assertEquals(ids, parsed.photoBlobIds)
    }

    @Test
    fun `keeps members that are missing from the local mirror`() {
        // The regression this whole design exists to prevent. The upload is built
        // from the album's stored membership, so members this device has never
        // synced (and therefore cannot resolve to a local photo) still appear in
        // the manifest. Building from the mirror instead is what let a
        // partially-synced phone replace an album with its own visible subset.
        val ids = listOf("synced-1", "never-synced-2", "never-synced-3", "synced-4")
        val payload = AlbumManifest.payloadFor(album(photoBlobIds = ids), null)

        assertEquals(ids, AlbumManifest.parse(payload).photoBlobIds)
    }

    @Test
    fun `a partially-synced device cannot shrink an album`() {
        // Sync a 5-member manifest down onto a device whose mirror holds 2 of
        // them, then re-upload: all 5 must survive the round trip.
        val server = listOf("b1", "b2", "b3", "b4", "b5")
        val downloaded = AlbumManifest.parse(
            AlbumManifest.build("a1", "Holiday", 1_700_000_000_000L, "b1", server),
        )

        // What syncAlbumsFromServer stores — verbatim, mirror untouched.
        val stored = album(id = "a1", name = "Holiday", photoBlobIds = downloaded.photoBlobIds)
        val reuploaded = AlbumManifest.parse(AlbumManifest.payloadFor(stored, "b1"))

        assertEquals(server, reuploaded.photoBlobIds)
    }

    @Test
    fun `writes the fields the web client reads`() {
        val payload = JSONObject(
            AlbumManifest.payloadFor(album(photoBlobIds = listOf("b1")), null),
        )
        assertEquals(1, payload.getInt("v"))
        assertEquals("album-1", payload.getString("album_id"))
        assertEquals("Trip to Rome", payload.getString("name"))
        // ISO-8601: web parses this with `new Date(...)`.
        assertEquals("2023-11-14T22:13:20Z", payload.getString("created_at"))
        assertTrue(payload.isNull("cover_photo_blob_id"))
        assertEquals(1, payload.getJSONArray("photo_blob_ids").length())
    }

    @Test
    fun `reads a manifest written by the web client`() {
        // Byte-for-byte the shape web's albumManifest.ts emits.
        val fromWeb = """
            {"v":1,"album_id":"a1","name":"Rome","created_at":"2023-11-14T22:13:20.000Z",
             "cover_photo_blob_id":"b1","photo_blob_ids":["b1","b2"]}
        """.trimIndent()

        val parsed = AlbumManifest.parse(fromWeb)
        assertEquals("a1", parsed.albumId)
        assertEquals("Rome", parsed.name)
        assertEquals(1_700_000_000_000L, parsed.createdAt)
        assertEquals("b1", parsed.coverPhotoBlobId)
        assertEquals(listOf("b1", "b2"), parsed.photoBlobIds)
    }

    @Test
    fun `tolerates a manifest with no members or cover`() {
        val parsed = AlbumManifest.parse(
            """{"v":1,"album_id":"a1","name":"Empty","created_at":"2023-11-14T22:13:20Z",
                "cover_photo_blob_id":null}""",
        )
        assertNull(parsed.coverPhotoBlobId)
        assertEquals(emptyList<String>(), parsed.photoBlobIds)
    }

    @Test
    fun `reports an unparsable created_at rather than inventing one`() {
        val parsed = AlbumManifest.parse(
            """{"v":1,"album_id":"a1","name":"X","created_at":"not-a-date","photo_blob_ids":[]}""",
        )
        assertNull(parsed.createdAt)
    }
}
