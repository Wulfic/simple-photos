package com.simplephotos.ui.components

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for [toggleGroupSelection] — the pure set-math behind the
 * "select / deselect a whole day at once" gesture (#24). Kept free of the
 * Compose runtime so it runs as a plain JVM test (matching the rest of the
 * local unit-test suite).
 */
class SelectionStateTest {

    @Test
    fun addsEntireGroupWhenNoneSelected() {
        val result = toggleGroupSelection(current = emptySet(), group = setOf("a", "b"))
        assertEquals(setOf("a", "b"), result)
    }

    @Test
    fun addsMissingMembersWhenGroupOnlyPartiallySelected() {
        // "b" is not selected yet, so the group is NOT fully selected → add all.
        val result = toggleGroupSelection(current = setOf("a"), group = setOf("a", "b"))
        assertEquals(setOf("a", "b"), result)
    }

    @Test
    fun removesEntireGroupWhenFullySelected() {
        // The core #24 fix: a fully-selected day must be removable in one tap.
        val result = toggleGroupSelection(current = setOf("a", "b", "c"), group = setOf("a", "b"))
        assertEquals(setOf("c"), result)
    }

    @Test
    fun removingTheOnlySelectedGroupYieldsEmpty() {
        val result = toggleGroupSelection(current = setOf("a", "b"), group = setOf("a", "b"))
        assertEquals(emptySet<String>(), result)
    }

    @Test
    fun addsGroupAlongsideAnExistingSelection() {
        val result = toggleGroupSelection(current = setOf("a", "b"), group = setOf("c", "d"))
        assertEquals(setOf("a", "b", "c", "d"), result)
    }

    @Test
    fun emptyGroupIsANoOp() {
        val result = toggleGroupSelection(current = setOf("a", "b"), group = emptySet())
        assertEquals(setOf("a", "b"), result)
    }

    @Test
    fun singleItemDayTogglesBothWays() {
        val added = toggleGroupSelection(current = emptySet(), group = setOf("a"))
        assertEquals(setOf("a"), added)
        val removed = toggleGroupSelection(current = added, group = setOf("a"))
        assertEquals(emptySet<String>(), removed)
    }

    // ── Compare selection gate (#21) ────────────────────────────────────────

    @Test
    fun canCompareOnlyWithExactlyTwo() {
        assertFalse(canCompare(0))
        assertFalse(canCompare(1))
        assertTrue(canCompare(2))
        assertFalse(canCompare(3))
        assertFalse(canCompare(10))
    }

    @Test
    fun compareTargetsReturnsOrderedPairForTwo() {
        assertEquals("a" to "b", compareTargets(listOf("a", "b")))
        // LinkedHashSet preserves insertion order (the selection's real type).
        assertEquals("x" to "y", compareTargets(linkedSetOf("x", "y")))
    }

    @Test
    fun compareTargetsIsNullUnlessExactlyTwo() {
        assertNull(compareTargets(emptyList()))
        assertNull(compareTargets(listOf("only")))
        assertNull(compareTargets(listOf("a", "b", "c")))
    }
}
