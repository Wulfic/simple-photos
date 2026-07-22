package com.simplephotos.data.album

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** A minimal stand-in for the fields the sort reads off a photo. */
private data class Item(val id: String, val takenAt: Long, val name: String)

private fun sort(items: List<Item>, s: AlbumSort): List<String> =
    sortAlbumItems(items, s, { it.takenAt }, { it.name }, { it.id }).map { it.id }

class AlbumSortTest {

    // ── Date ────────────────────────────────────────────────────────────────

    @Test
    fun `date desc puts the newest capture first`() {
        val items = listOf(
            Item("old", 100, "a"),
            Item("new", 300, "b"),
            Item("mid", 200, "c"),
        )
        assertEquals(
            listOf("new", "mid", "old"),
            sort(items, AlbumSort(AlbumSortField.DATE, AlbumSortDir.DESC)),
        )
    }

    @Test
    fun `date asc puts the oldest capture first`() {
        val items = listOf(
            Item("old", 100, "a"),
            Item("new", 300, "b"),
            Item("mid", 200, "c"),
        )
        assertEquals(
            listOf("old", "mid", "new"),
            sort(items, AlbumSort(AlbumSortField.DATE, AlbumSortDir.ASC)),
        )
    }

    @Test
    fun `a missing takenAt sorts as the oldest`() {
        val items = listOf(Item("has", 500, "a"), Item("missing", 0, "b"))
        assertEquals(
            listOf("has", "missing"),
            sort(items, AlbumSort(AlbumSortField.DATE, AlbumSortDir.DESC)),
        )
        assertEquals(
            listOf("missing", "has"),
            sort(items, AlbumSort(AlbumSortField.DATE, AlbumSortDir.ASC)),
        )
    }

    // ── Name (natural / numeric-aware) ────────────────────────────────────────

    @Test
    fun `name asc orders numerically so IMG_2 precedes IMG_10`() {
        val items = listOf(
            Item("x", 0, "IMG_10.jpg"),
            Item("y", 0, "IMG_2.jpg"),
            Item("z", 0, "IMG_1.jpg"),
        )
        assertEquals(
            listOf("z", "y", "x"),
            sort(items, AlbumSort(AlbumSortField.NAME, AlbumSortDir.ASC)),
        )
    }

    @Test
    fun `name sort is case-insensitive`() {
        val items = listOf(Item("x", 0, "banana.jpg"), Item("y", 0, "Apple.jpg"))
        assertEquals(
            listOf("y", "x"),
            sort(items, AlbumSort(AlbumSortField.NAME, AlbumSortDir.ASC)),
        )
    }

    @Test
    fun `name desc reverses the order`() {
        val items = listOf(Item("a", 0, "a.jpg"), Item("b", 0, "b.jpg"), Item("c", 0, "c.jpg"))
        assertEquals(
            listOf("c", "b", "a"),
            sort(items, AlbumSort(AlbumSortField.NAME, AlbumSortDir.DESC)),
        )
    }

    @Test
    fun `naturalCompare handles leading zeros and equal numbers`() {
        // 007 == 7 numerically (leading zeros ignored).
        assertEquals(0, naturalCompare("img007.jpg", "img7.jpg"))
        // 2 < 10 numerically, not lexically.
        assert(naturalCompare("v2", "v10") < 0)
        // Pure text still compares case-folded.
        assert(naturalCompare("apple", "banana") < 0)
    }

    // ── Deterministic ties ────────────────────────────────────────────────────

    @Test
    fun `a date tie breaks on id, stable regardless of input order`() {
        val a = Item("zeta", 100, "z")
        val b = Item("alpha", 100, "a")
        val c = Item("mu", 100, "m")
        val expected = listOf("alpha", "mu", "zeta")
        assertEquals(expected, sort(listOf(a, b, c), AlbumSort(AlbumSortField.DATE, AlbumSortDir.ASC)))
        assertEquals(expected, sort(listOf(c, a, b), AlbumSort(AlbumSortField.DATE, AlbumSortDir.ASC)))
    }

    // ── Toggle logic ──────────────────────────────────────────────────────────

    @Test
    fun `tapping the active field reverses its direction`() {
        val start = AlbumSort(AlbumSortField.DATE, AlbumSortDir.DESC)
        assertEquals(
            AlbumSort(AlbumSortField.DATE, AlbumSortDir.ASC),
            nextSort(start, AlbumSortField.DATE),
        )
    }

    @Test
    fun `tapping a different field switches to its natural direction`() {
        val start = AlbumSort(AlbumSortField.DATE, AlbumSortDir.DESC)
        // Name defaults to ASC (A→Z); Date defaults to DESC (newest first).
        assertEquals(
            AlbumSort(AlbumSortField.NAME, AlbumSortDir.ASC),
            nextSort(start, AlbumSortField.NAME),
        )
        assertEquals(
            AlbumSort(AlbumSortField.DATE, AlbumSortDir.DESC),
            nextSort(AlbumSort(AlbumSortField.NAME, AlbumSortDir.ASC), AlbumSortField.DATE),
        )
    }

    @Test
    fun `defaultDirFor dates desc and names asc`() {
        assertEquals(AlbumSortDir.DESC, defaultDirFor(AlbumSortField.DATE))
        assertEquals(AlbumSortDir.ASC, defaultDirFor(AlbumSortField.NAME))
    }

    // ── Persistence serialization ─────────────────────────────────────────────

    @Test
    fun `serialize round-trips through parse`() {
        for (field in AlbumSortField.entries) {
            for (dir in AlbumSortDir.entries) {
                val s = AlbumSort(field, dir)
                assertEquals(s, parseAlbumSort(s.serialize()))
            }
        }
    }

    @Test
    fun `parse treats missing or malformed values as no choice`() {
        assertNull(parseAlbumSort(null))
        assertNull(parseAlbumSort(""))
        assertNull(parseAlbumSort("garbage"))
        assertNull(parseAlbumSort("size:asc"))
        assertNull(parseAlbumSort("date:sideways"))
        assertNull(parseAlbumSort("date"))
    }
}
