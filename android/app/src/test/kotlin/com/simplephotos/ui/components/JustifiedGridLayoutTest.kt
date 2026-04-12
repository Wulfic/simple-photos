package com.simplephotos.ui.components

import org.junit.Assert.*
import org.junit.Test

/**
 * Unit tests for the JustifiedGrid layout algorithm.
 *
 * The computeRows function is the core layout engine — it packs items
 * into rows with proportional widths based on aspect ratios, similar
 * to the web's JustifiedGrid.tsx.
 */
class JustifiedGridLayoutTest {

    /**
     * Mirror of the production computeRows algorithm from JustifiedGrid.kt.
     * Kept here to avoid depending on Compose runtime.
     */
    private data class LayoutRow(
        val startIdx: Int,
        val count: Int,
        val height: Float,
        val isFull: Boolean
    )

    private fun computeRows(
        aspectRatios: List<Float>,
        containerWidth: Float,
        targetRowHeight: Float,
        gap: Float,
        breakBefore: Set<Int> = emptySet()
    ): List<LayoutRow> {
        if (aspectRatios.isEmpty() || containerWidth <= 0f) return emptyList()
        val rows = mutableListOf<LayoutRow>()
        var rowStart = 0
        var rowAspectSum = 0f

        for (i in aspectRatios.indices) {
            // Force-break: flush the current row before this item
            if (i in breakBefore && i > rowStart) {
                val count = i - rowStart
                val totalGap = (count - 1) * gap
                val naturalWidth = rowAspectSum * targetRowHeight + totalGap
                val isFull = naturalWidth >= containerWidth
                val rowHeight = if (isFull) {
                    (containerWidth - totalGap) / rowAspectSum
                } else {
                    incompleteRowHeight(rowAspectSum, totalGap, containerWidth, targetRowHeight)
                }
                rows.add(LayoutRow(startIdx = rowStart, count = count, height = rowHeight, isFull = isFull))
                rowStart = i
                rowAspectSum = 0f
            }

            rowAspectSum += aspectRatios[i]
            val itemCount = i - rowStart + 1
            val totalGap = (itemCount - 1) * gap
            val naturalWidth = rowAspectSum * targetRowHeight + totalGap

            if (naturalWidth >= containerWidth) {
                val availableWidth = containerWidth - totalGap
                val rowHeight = availableWidth / rowAspectSum
                rows.add(LayoutRow(startIdx = rowStart, count = itemCount, height = rowHeight, isFull = true))
                rowStart = i + 1
                rowAspectSum = 0f
            }
        }

        // Last incomplete row — boost height so items span at least 35% of container
        if (rowStart < aspectRatios.size) {
            val count = aspectRatios.size - rowStart
            val lastAspects = aspectRatios.subList(rowStart, aspectRatios.size)
            val aspectSum = lastAspects.sum()
            val gapTotal = (count - 1) * gap
            val rowHeight = incompleteRowHeight(aspectSum, gapTotal, containerWidth, targetRowHeight)
            rows.add(LayoutRow(
                startIdx = rowStart,
                count = count,
                height = rowHeight,
                isFull = false
            ))
        }

        return rows
    }

    private fun incompleteRowHeight(
        aspectSum: Float,
        totalGapPx: Float,
        containerWidth: Float,
        targetRowHeight: Float
    ): Float {
        val minWidth = containerWidth * 0.35f
        val desiredHeight = (minWidth - totalGapPx) / aspectSum
        val maxHeight = targetRowHeight * 2f
        return desiredHeight.coerceIn(targetRowHeight, maxHeight)
    }

    @Test
    fun emptyList_returnsNoRows() {
        val rows = computeRows(emptyList(), 1080f, 240f, 4f)
        assertTrue(rows.isEmpty())
    }

    @Test
    fun singleItem_returnsOneRow() {
        val rows = computeRows(listOf(1.5f), 1080f, 240f, 4f)
        assertEquals(1, rows.size)
        assertEquals(0, rows[0].startIdx)
        assertEquals(1, rows[0].count)
    }

    @Test
    fun allSquareItems_packsMultiplePerRow() {
        // Five 1:1 items at 1080px wide, target 240px row height
        val aspects = List(5) { 1.0f }
        val rows = computeRows(aspects, 1080f, 240f, 4f)
        assertTrue(rows.isNotEmpty())
        assertEquals(5, rows.sumOf { it.count })
    }

    @Test
    fun mixedAspectRatios_respectsOrder() {
        val aspects = listOf(0.5f, 1.0f, 2.0f, 0.75f, 1.33f)
        val rows = computeRows(aspects, 1080f, 240f, 4f)
        assertEquals(5, rows.sumOf { it.count })
        var expectedStart = 0
        for (row in rows) {
            assertEquals(expectedStart, row.startIdx)
            expectedStart += row.count
        }
    }

    @Test
    fun wideContainer_fitsMorePerRow() {
        val aspects = List(10) { 1.5f }
        val narrowRows = computeRows(aspects, 540f, 200f, 4f)
        val wideRows = computeRows(aspects, 1080f, 200f, 4f)
        assertTrue(wideRows.size <= narrowRows.size)
    }

    @Test
    fun extremeAspectRatios_handled() {
        // Very thin portrait (0.1) and very wide panorama (10.0)
        val aspects = listOf(0.1f, 10.0f)
        val rows = computeRows(aspects, 1080f, 240f, 4f)
        assertEquals(2, rows.sumOf { it.count })
    }

    @Test
    fun rowHeight_withinReasonableRange() {
        val aspects = listOf(1.5f, 1.0f, 2.0f)
        val targetH = 240f
        val rows = computeRows(aspects, 1080f, targetH, 4f)
        for (row in rows) {
            assertTrue("Row height ${row.height} too small", row.height > targetH * 0.3f)
            assertTrue("Row height ${row.height} too large", row.height < targetH * 3f)
        }
    }

    @Test
    fun portraitPhotos_allAccountedFor() {
        val aspects = listOf(0.67f, 0.75f, 0.56f, 0.67f, 0.75f)
        val rows = computeRows(aspects, 1080f, 240f, 4f)
        assertEquals(5, rows.sumOf { it.count })
    }

    // ── breakBefore tests ───────────────────────────────────────────────────

    @Test
    fun breakBefore_forcesNewRow() {
        // 6 square items, break before index 3 → should get at least 2 logical groups
        val aspects = List(6) { 1.0f }
        val rows = computeRows(aspects, 1080f, 240f, 4f, breakBefore = setOf(3))
        assertEquals(6, rows.sumOf { it.count })
        // Index 3 must be the start of a row
        assertTrue("Index 3 should start a row", rows.any { it.startIdx == 3 })
    }

    @Test
    fun breakBefore_atZero_noEffect() {
        // Break at index 0 should not create an empty row
        val aspects = List(4) { 1.0f }
        val rows = computeRows(aspects, 1080f, 240f, 4f, breakBefore = setOf(0))
        assertEquals(4, rows.sumOf { it.count })
        assertTrue(rows.first().startIdx == 0)
        assertTrue(rows.first().count > 0)
    }

    @Test
    fun breakBefore_multipleBreaks() {
        // 9 items with breaks at 3 and 6 → 3 groups
        val aspects = List(9) { 1.0f }
        val rows = computeRows(aspects, 1080f, 240f, 4f, breakBefore = setOf(3, 6))
        assertEquals(9, rows.sumOf { it.count })
        assertTrue("Index 3 should start a row", rows.any { it.startIdx == 3 })
        assertTrue("Index 6 should start a row", rows.any { it.startIdx == 6 })
    }

    @Test
    fun breakBefore_flushedRowRespectstargetHeight() {
        // Break forces a short row — its height should be boosted but capped at 2× target
        val aspects = listOf(0.5f, 0.5f, 1.0f, 1.0f, 1.0f)
        val targetH = 240f
        val rows = computeRows(aspects, 1080f, targetH, 4f, breakBefore = setOf(2))
        // The first row (items 0-1) is flushed short — height ≤ 2× target
        val firstRow = rows.first()
        assertEquals(0, firstRow.startIdx)
        assertTrue(firstRow.count <= 2)
        assertFalse("Flushed incomplete row should not be full", firstRow.isFull)
        assertTrue("Flushed row height should not exceed 2× target", firstRow.height <= targetH * 2f)
    }
}
