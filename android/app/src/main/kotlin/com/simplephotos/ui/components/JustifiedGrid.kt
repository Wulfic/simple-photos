/**
 * Justified flex-row grid — Google-Photos-style layout where photos maintain
 * their natural aspect ratios.  Each row is filled by items whose widths are
 * proportional to their aspect ratio so the row fills the container width
 * exactly.  The last (incomplete) row uses the target height without stretching.
 *
 * Port of the web JustifiedGrid.tsx compute-rows algorithm.
 */
package com.simplephotos.ui.components

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// ── Data types ──────────────────────────────────────────────────────────────

internal data class LayoutRow(
    val startIdx: Int,
    val count: Int,
    val height: Float,   // px
    val isFull: Boolean  // true when items naturally filled the container width
)

/**
 * Compute justified rows using a greedy algorithm.
 *
 * For each row the total "natural width at targetHeight" of accumulated items
 * is tracked.  Once it exceeds [containerWidthPx] the row is closed and its
 * actual height shrunk so everything fits exactly.  The last row keeps the
 * target height and is left-aligned.
 *
 * @param breakBefore   Set of item indices that MUST start a new row (e.g. day
 *                      group boundaries).  The row currently being built is
 *                      flushed before these indices.
 */
internal fun computeRows(
    aspectRatios: List<Float>,
    containerWidthPx: Float,
    targetRowHeightPx: Float,
    gapPx: Float,
    breakBefore: Set<Int> = emptySet()
): List<LayoutRow> {
    if (containerWidthPx <= 0f || aspectRatios.isEmpty()) return emptyList()

    val rows = mutableListOf<LayoutRow>()
    var rowStart = 0
    var rowAspectSum = 0f

    for (i in aspectRatios.indices) {
        // Force-break: flush the current row before this item
        if (i in breakBefore && i > rowStart) {
            val count = i - rowStart
            val totalGap = (count - 1) * gapPx
            val naturalWidth = rowAspectSum * targetRowHeightPx + totalGap
            val isFull = naturalWidth >= containerWidthPx
            val rowHeight = if (isFull) {
                (containerWidthPx - totalGap) / rowAspectSum
            } else {
                incompleteRowHeight(rowAspectSum, totalGap, containerWidthPx, targetRowHeightPx)
            }
            rows.add(LayoutRow(startIdx = rowStart, count = count, height = rowHeight, isFull = isFull))
            rowStart = i
            rowAspectSum = 0f
        }

        rowAspectSum += aspectRatios[i]
        val itemCount = i - rowStart + 1
        val totalGap = (itemCount - 1) * gapPx
        val naturalWidth = rowAspectSum * targetRowHeightPx + totalGap

        if (naturalWidth >= containerWidthPx) {
            val availableWidth = containerWidthPx - totalGap
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
        val gapTotal = (count - 1) * gapPx
        val rowHeight = incompleteRowHeight(aspectSum, gapTotal, containerWidthPx, targetRowHeightPx)
        rows.add(LayoutRow(
            startIdx = rowStart,
            count = count,
            height = rowHeight,
            isFull = false
        ))
    }

    return rows
}

/**
 * Compute a boosted row height for incomplete rows so items span at least
 * 35 % of the container width, capped at 2× the target height.
 */
private fun incompleteRowHeight(
    aspectSum: Float,
    totalGapPx: Float,
    containerWidthPx: Float,
    targetRowHeightPx: Float
): Float {
    val minWidth = containerWidthPx * 0.35f
    val desiredHeight = (minWidth - totalGapPx) / aspectSum
    val maxHeight = targetRowHeightPx * 2f
    return desiredHeight.coerceIn(targetRowHeightPx, maxHeight)
}

// ── Composable ──────────────────────────────────────────────────────────────

/**
 * A Google-Photos-style justified grid that lays out items in rows,
 * maintaining natural aspect ratios with no square cropping.
 *
 * @param items         Items to lay out.
 * @param getAspectRatio Extract width/height ratio from an item (default 1f).
 * @param getKey        Stable unique key per item.
 * @param targetRowHeight Desired row height — actual height varies to fill width.
 * @param gap           Gap between items and rows.
 * @param breakBefore   Set of item indices that must start a new row (e.g. day
 *                      group boundaries). Headers are emitted for these indices.
 * @param headerBefore  Optional composable for section headers; receives the
 *                      item index and should emit content only when appropriate.
 * @param listState     Optional LazyListState for scroll position restoration.
 * @param itemContent   Content for a single item; receives the item, width in dp,
 *                      and height in dp so it can size itself.
 */
@Composable
fun <T> JustifiedGrid(
    items: List<T>,
    getAspectRatio: (T) -> Float,
    getKey: (T) -> Any,
    targetRowHeight: Dp = 180.dp,
    gap: Dp = 4.dp,
    breakBefore: Set<Int> = emptySet(),
    headerBefore: (@Composable (itemIndex: Int) -> Unit)? = null,
    listState: LazyListState = rememberLazyListState(),
    itemContent: @Composable (item: T, widthDp: Dp, heightDp: Dp) -> Unit
) {
    val density = LocalDensity.current
    val configuration = LocalConfiguration.current
    val screenWidthPx = with(density) { configuration.screenWidthDp.dp.toPx() }
    val gapPx = with(density) { gap.toPx() }
    val targetRowHeightPx = with(density) { targetRowHeight.toPx() }

    // Clamp aspect ratios to [0.3, 4.0] to avoid degenerate rows
    val aspectRatios = remember(items) {
        items.map { getAspectRatio(it).coerceIn(0.3f, 4.0f) }
    }

    val rows = remember(aspectRatios, screenWidthPx, targetRowHeightPx, gapPx, breakBefore) {
        computeRows(aspectRatios, screenWidthPx, targetRowHeightPx, gapPx, breakBefore)
    }

    // Build a flat list of "render entries" so headers and rows each get a
    // LazyColumn slot.  This collapses to O(rows + headers).
    data class RenderEntry(
        val isHeader: Boolean,
        val headerItemIndex: Int = 0,
        val row: LayoutRow? = null,
        val rowAspects: List<Float> = emptyList()
    )

    val renderEntries = remember(rows, aspectRatios, breakBefore) {
        val entries = mutableListOf<RenderEntry>()
        for (row in rows) {
            // Emit header entries only for items that start a new group
            if (headerBefore != null && row.startIdx in breakBefore) {
                entries.add(RenderEntry(isHeader = true, headerItemIndex = row.startIdx))
            }
            entries.add(RenderEntry(
                isHeader = false,
                row = row,
                rowAspects = aspectRatios.subList(row.startIdx, row.startIdx + row.count)
            ))
        }
        entries
    }

    LazyColumn(
        state = listState,
        modifier = Modifier.fillMaxWidth()
    ) {
        items(
            count = renderEntries.size,
            key = { idx ->
                val entry = renderEntries[idx]
                if (entry.isHeader) "header_${entry.headerItemIndex}"
                else "row_${entry.row!!.startIdx}"
            }
        ) { idx ->
            val entry = renderEntries[idx]
            if (entry.isHeader && headerBefore != null) {
                headerBefore(entry.headerItemIndex)
            } else if (entry.row != null) {
                val row = entry.row
                val rowHeightDp = with(density) { row.height.toDp() }
                val isFullRow = row.isFull

                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(rowHeightDp)
                ) {
                    for (i in 0 until row.count) {
                        val globalIdx = row.startIdx + i
                        val item = items[globalIdx]
                        val ar = entry.rowAspects[i]

                        if (i > 0) {
                            Spacer(Modifier.width(gap))
                        }

                        val itemWidthDp = if (isFullRow) {
                            // Full row: width proportional to aspect ratio
                            // We calculate the exact pixel width and convert
                            val totalGapPx = (row.count - 1) * gapPx
                            val availableWidthPx = screenWidthPx - totalGapPx
                            val rowAspectSum = entry.rowAspects.sum()
                            val itemWidthPx = (ar / rowAspectSum) * availableWidthPx
                            with(density) { itemWidthPx.toDp() }
                        } else {
                            // Last row: fixed width based on aspect ratio × row height
                            with(density) { (ar * row.height).toDp() }
                        }

                        Box(
                            modifier = Modifier
                                .width(itemWidthDp)
                                .height(rowHeightDp)
                        ) {
                            itemContent(item, itemWidthDp, rowHeightDp)
                        }
                    }
                }
                Spacer(Modifier.height(gap))
            }
        }
    }
}
