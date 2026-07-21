package com.simplephotos.data.local

import com.simplephotos.data.media.Rendition
import com.simplephotos.data.media.offerableRenditions
import com.simplephotos.data.media.shouldOfferPicker
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The converter behind `AlbumEntity.photoBlobIds` — an album's membership of
 * record. Anything it loses on the way through SQLite is membership silently
 * dropped from the album on the next manifest upload.
 */
class ConvertersTest {
    private val converters = Converters()

    private fun roundTrip(value: List<String>): List<String> =
        converters.jsonToStringList(converters.stringListToJson(value))

    @Test
    fun `round-trips a membership list in order`() {
        val ids = listOf("blob-1", "blob-2", "blob-3")
        assertEquals(ids, roundTrip(ids))
    }

    @Test
    fun `round-trips an empty list`() {
        assertEquals(emptyList<String>(), roundTrip(emptyList()))
    }

    @Test
    fun `survives ids containing separator characters`() {
        // Blob ids are opaque server strings. This is exactly the case a
        // delimiter-joined encoding would corrupt — one member splitting into
        // two, with nothing downstream able to tell.
        val ids = listOf("a,b", "c\"d", "e]f[g", "h\\i")
        assertEquals(ids, roundTrip(ids))
    }

    @Test
    fun `reads a legacy or empty column as no members`() {
        assertEquals(emptyList<String>(), converters.jsonToStringList(""))
    }

    // ── #49 resolution ladder ───────────────────────────────────────────────

    private fun rung(
        shortEdge: Int,
        isSource: Boolean = false,
        blobId: String? = "b-$shortEdge",
        codec: String? = "h264",
    ) = Rendition(
        shortEdge = shortEdge,
        width = shortEdge * 16 / 9,
        height = shortEdge,
        isSource = isSource,
        blobId = blobId,
        codec = codec,
        sizeBytes = shortEdge * 1000L,
    )

    private fun roundTripLadder(value: List<Rendition>): List<Rendition> =
        converters.jsonToRenditionList(converters.renditionListToJson(value))

    @Test
    fun `round-trips a ladder in order`() {
        val ladder = listOf(rung(2160, isSource = true), rung(1080))
        assertEquals(ladder, roundTripLadder(ladder))
    }

    @Test
    fun `round-trips an empty ladder`() {
        // The normal case: only 136 of 742 live videos ever get a second rung.
        assertEquals(emptyList<Rendition>(), roundTripLadder(emptyList()))
    }

    @Test
    fun `reads a legacy or empty column as no ladder`() {
        // Pre-#49 rows and anything unparseable must degrade to "one quality,
        // no picker" rather than taking the gallery down.
        assertEquals(emptyList<Rendition>(), converters.jsonToRenditionList(""))
        assertEquals(emptyList<Rendition>(), converters.jsonToRenditionList("{not json"))
    }

    @Test
    fun `preserves a null blob id as null rather than empty string`() {
        // The trap: JSONObject.optString returns "" for an absent key, and ""
        // is not null — so an unencrypted install's rungs would survive the
        // picker's `blobId != null` filter and then build a hostless
        // `spblob://` URI that throws at playback. Round-tripping must keep
        // null genuinely null.
        val ladder = listOf(rung(2160, isSource = true, blobId = null, codec = null))
        val got = roundTripLadder(ladder)
        assertNull(got.single().blobId)
        assertNull(got.single().codec)
        assertEquals(ladder, got)
    }

    @Test
    fun `a stored ladder still filters and sorts correctly after a round trip`() {
        // Guards the seam between storage and the picker: a ladder that came
        // back from SQLite must behave exactly like one straight off the wire.
        val stored = roundTripLadder(
            listOf(rung(1080), rung(2160, isSource = true), rung(720, blobId = null))
        )
        assertTrue(shouldOfferPicker(stored))
        assertEquals(listOf(2160, 1080), offerableRenditions(stored).map { it.shortEdge })
    }
}
