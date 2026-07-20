/**
 * JustifiedGrid windowing (#51).
 *
 * The property that matters: the number of mounted rows is bounded by the
 * viewport, not by the length of the list. A 10k-photo library must mount the
 * same handful of rows a 100-photo one does.
 */
import { describe, it, expect } from "vitest";
import { computeRowOffsets, computeGridWindow, type LayoutRow } from "./gridWindow";

const GAP = 4;
const ROW_H = 200;

/** `n` uniform rows of 4 items each. */
function rows(n: number, height = ROW_H): LayoutRow[] {
  return Array.from({ length: n }, (_, i) => ({
    startIdx: i * 4,
    count: 4,
    height,
  }));
}

/** Total laid-out height, including each row's trailing gap. */
const totalOf = (offsets: number[]) => offsets[offsets.length - 1];

describe("computeRowOffsets", () => {
  it("returns rowCount + 1 entries with the total last", () => {
    const offsets = computeRowOffsets(rows(3), GAP);
    expect(offsets).toEqual([0, 204, 408, 612]);
  });

  it("handles an empty grid", () => {
    expect(computeRowOffsets([], GAP)).toEqual([0]);
  });

  it("accumulates variable row heights", () => {
    const varied: LayoutRow[] = [
      { startIdx: 0, count: 2, height: 100 },
      { startIdx: 2, count: 3, height: 250 },
      { startIdx: 5, count: 1, height: 50 },
    ];
    expect(computeRowOffsets(varied, GAP)).toEqual([0, 104, 358, 412]);
  });
});

describe("computeGridWindow — bounded mounting", () => {
  it("mounts a viewport-sized slice of a 10k-photo grid, not the whole thing", () => {
    const all = rows(2500); // 2500 rows × 4 = 10,000 photos
    const offsets = computeRowOffsets(all, GAP);
    const win = computeGridWindow(offsets, 0, 900, 450);

    const mountedRows = win.endRow - win.startRow;
    expect(mountedRows).toBeLessThan(12);
    // 4 items per row — the todo's "< ~200 mounted tiles" bound, with room to spare.
    expect(mountedRows * 4).toBeLessThan(200);
  });

  it("mounts the same number of rows regardless of list length", () => {
    const windowFor = (n: number) => {
      const offsets = computeRowOffsets(rows(n), GAP);
      const w = computeGridWindow(offsets, 5000, 5900, 450);
      return w.endRow - w.startRow;
    };
    expect(windowFor(2500)).toBe(windowFor(100_000));
  });

  it("scrolling deep into the list keeps the slice bounded", () => {
    const offsets = computeRowOffsets(rows(2500), GAP);
    for (let top = 0; top < totalOf(offsets); top += 5000) {
      const win = computeGridWindow(offsets, top, top + 900, 450);
      expect(win.endRow - win.startRow).toBeLessThan(12);
    }
  });
});

describe("computeGridWindow — scroll height is preserved", () => {
  const offsets = computeRowOffsets(rows(500), GAP);
  const total = totalOf(offsets);

  it("keeps padTop + rendered + padBottom === total at every scroll offset", () => {
    for (let top = -2000; top < total + 2000; top += 137) {
      const win = computeGridWindow(offsets, top, top + 800, 300);
      const rendered = offsets[win.endRow] - offsets[win.startRow];
      expect(win.padTop + rendered + win.padBottom).toBe(total);
    }
  });

  it("renders nothing but reserves full height when the grid is below the viewport", () => {
    const win = computeGridWindow(offsets, -50_000, -40_000, 0);
    expect(win.endRow - win.startRow).toBe(0);
    expect(win.padTop).toBe(0);
    expect(win.padBottom).toBe(total);
  });

  it("renders nothing but reserves full height when the grid is above the viewport", () => {
    const win = computeGridWindow(offsets, total + 10_000, total + 20_000, 0);
    expect(win.endRow - win.startRow).toBe(0);
    expect(win.padTop).toBe(total);
    expect(win.padBottom).toBe(0);
  });

  it("handles an empty grid without producing phantom height", () => {
    const win = computeGridWindow(computeRowOffsets([], GAP), 0, 800, 300);
    expect(win).toEqual({ startRow: 0, endRow: 0, padTop: 0, padBottom: 0 });
  });
});

describe("computeGridWindow — correctness of the slice", () => {
  const offsets = computeRowOffsets(rows(100), GAP); // rows at 0,204,408,...

  it("includes every row that intersects the viewport", () => {
    const win = computeGridWindow(offsets, 500, 1000, 0);
    // Row i spans [204i, 204i+200). Viewport 500..1000 touches rows 2..4.
    expect(win.startRow).toBe(2);
    expect(win.endRow).toBe(5);
  });

  it("grows the slice by the overscan band on both sides", () => {
    const tight = computeGridWindow(offsets, 500, 1000, 0);
    const loose = computeGridWindow(offsets, 500, 1000, 400);
    expect(loose.startRow).toBeLessThan(tight.startRow);
    expect(loose.endRow).toBeGreaterThan(tight.endRow);
  });

  it("starts at row 0 at the top of the list", () => {
    const win = computeGridWindow(offsets, 0, 800, 300);
    expect(win.startRow).toBe(0);
    expect(win.padTop).toBe(0);
  });

  it("reaches the final row at the bottom of the list", () => {
    const total = totalOf(offsets);
    const win = computeGridWindow(offsets, total - 800, total, 300);
    expect(win.endRow).toBe(100);
    expect(win.padBottom).toBe(0);
  });

  it("never returns an inverted range for a degenerate viewport", () => {
    const win = computeGridWindow(offsets, 1000, 200, 0);
    expect(win.endRow).toBeGreaterThanOrEqual(win.startRow);
    expect(win.padTop + win.padBottom).toBe(totalOf(offsets));
  });
});
