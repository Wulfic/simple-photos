/**
 * Windowing math for {@link JustifiedGrid} (#51).
 *
 * The grid used to mount a DOM node for every photo it had ever been handed —
 * 10k+ `<div>` + `<img>` pairs live at once on a large library, which is the
 * crash. Row heights are already known up front from the layout pass, so the
 * grid can render only the rows intersecting the viewport and pad the rest with
 * two spacer divs.
 *
 * These functions are pure and live apart from the component precisely so the
 * bound on mounted tiles is a unit-testable property rather than something you
 * have to drive a browser to observe.
 *
 * Coordinates are **grid-local**: 0 is the top of the grid container, not the
 * top of the document. The component converts by reading its own bounding rect,
 * which keeps it correct no matter what headers or banners sit above it.
 */

export interface LayoutRow {
  startIdx: number;
  count: number;
  height: number;
}

export interface GridWindow {
  /** First row to render, inclusive. */
  startRow: number;
  /** One past the last row to render. */
  endRow: number;
  /** Spacer height above the rendered rows, in px. */
  padTop: number;
  /** Spacer height below the rendered rows, in px. */
  padBottom: number;
}

/**
 * Cumulative top offset of every row, plus a final entry holding the grid's
 * total height.
 *
 * Length is `rows.length + 1`, so `offsets[i]` is row `i`'s top edge and
 * `offsets[i + 1]` its bottom edge. Each row contributes its height plus `gap`,
 * matching the `marginBottom` the component renders — including on the last
 * row, so the total is exactly the height the browser lays out. Getting this
 * wrong would change the document height and break scroll restoration.
 */
export function computeRowOffsets(rows: LayoutRow[], gap: number): number[] {
  const offsets = new Array<number>(rows.length + 1);
  let acc = 0;
  offsets[0] = 0;
  for (let i = 0; i < rows.length; i++) {
    acc += rows[i].height + gap;
    offsets[i + 1] = acc;
  }
  return offsets;
}

/** Smallest `i` in `[0, n)` with `offsets[i + 1] > edge`, else `n`. */
function firstRowEndingAfter(offsets: number[], edge: number): number {
  let lo = 0;
  let hi = offsets.length - 1; // exclusive row count
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offsets[mid + 1] > edge) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

/** Smallest `i` in `[0, n]` with `offsets[i] >= edge`, else `n`. */
function firstRowStartingAtOrAfter(offsets: number[], edge: number): number {
  let lo = 0;
  let hi = offsets.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offsets[mid] >= edge) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

/**
 * Select the rows intersecting `[viewportTop, viewportBottom]` grown by
 * `overscanPx` on each side.
 *
 * Invariant, and the thing worth testing: `padTop + renderedHeight + padBottom`
 * always equals the grid's total height, for every scroll position — including
 * when the grid is entirely off-screen and nothing is rendered at all. Scroll
 * height must not depend on what happens to be mounted.
 */
export function computeGridWindow(
  offsets: number[],
  viewportTop: number,
  viewportBottom: number,
  overscanPx: number,
): GridWindow {
  const rowCount = offsets.length - 1;
  const total = rowCount > 0 ? offsets[rowCount] : 0;
  if (rowCount <= 0) {
    return { startRow: 0, endRow: 0, padTop: 0, padBottom: 0 };
  }

  const top = viewportTop - overscanPx;
  const bottom = viewportBottom + overscanPx;

  // Degenerate viewport (bottom above top) — render nothing but keep the
  // spacers summing to the full height.
  if (bottom <= top) {
    return { startRow: 0, endRow: 0, padTop: 0, padBottom: total };
  }

  const startRow = firstRowEndingAfter(offsets, top);
  const endRow = Math.max(startRow, firstRowStartingAtOrAfter(offsets, bottom));

  return {
    startRow,
    endRow,
    padTop: offsets[startRow],
    padBottom: total - offsets[endRow],
  };
}
