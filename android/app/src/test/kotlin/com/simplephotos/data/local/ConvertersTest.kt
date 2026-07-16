package com.simplephotos.data.local

import org.junit.Assert.assertEquals
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
}
