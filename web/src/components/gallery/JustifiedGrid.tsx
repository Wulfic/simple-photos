/**
 * Justified flex-row grid — Google-Photos-style layout where photos maintain
 * their natural aspect ratios.  Each row is a flex container whose children
 * grow proportional to their aspect ratio so the row fills the container width
 * exactly.  The last (incomplete) row uses the target height without stretching.
 */
import { useState, useEffect, useRef, useMemo, type ReactNode } from "react";
import { useThumbnailSizeStore } from "../../store/thumbnailSize";

export interface JustifiedGridProps<T> {
  /** Items to lay out. */
  items: T[];
  /** Extract aspect ratio (width / height) from an item.  Return 1 for unknowns. */
  getAspectRatio: (item: T) => number;
  /** Render a single item.  The rendered element should be w-full h-full. */
  renderItem: (item: T, index: number) => ReactNode;
  /** Unique key per item. */
  getKey: (item: T) => string;
  /** Gap between items in pixels (default 4). */
  gap?: number;
}

interface LayoutRow {
  startIdx: number;
  count: number;
  height: number;
}

/**
 * Compute justified rows using a greedy algorithm.
 *
 * For each row the total "natural width at targetHeight" of accumulated items
 * is tracked.  Once it exceeds `containerWidth` the row is closed and its
 * actual height shrunk so everything fits exactly.  The last row keeps the
 * target height and is left-aligned.
 */
function computeRows(
  aspectRatios: number[],
  containerWidth: number,
  targetRowHeight: number,
  gap: number,
): LayoutRow[] {
  if (containerWidth <= 0 || aspectRatios.length === 0) return [];

  const rows: LayoutRow[] = [];
  let rowStart = 0;
  let rowAspectSum = 0;

  for (let i = 0; i < aspectRatios.length; i++) {
    rowAspectSum += aspectRatios[i];
    const itemCount = i - rowStart + 1;
    const totalGap = (itemCount - 1) * gap;
    const naturalWidth = rowAspectSum * targetRowHeight + totalGap;

    if (naturalWidth >= containerWidth) {
      // Row is full — compute exact height so items fill containerWidth
      const availableWidth = containerWidth - totalGap;
      const rowHeight = availableWidth / rowAspectSum;
      rows.push({ startIdx: rowStart, count: itemCount, height: rowHeight });
      rowStart = i + 1;
      rowAspectSum = 0;
    }
  }

  // Last incomplete row — keep target height, left-aligned
  if (rowStart < aspectRatios.length) {
    rows.push({
      startIdx: rowStart,
      count: aspectRatios.length - rowStart,
      height: targetRowHeight,
    });
  }

  return rows;
}

export default function JustifiedGrid<T>({
  items,
  getAspectRatio,
  renderItem,
  getKey,
  gap = 4,
}: JustifiedGridProps<T>) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(0);
  const targetRowHeight = useThumbnailSizeStore((s) => s.targetRowHeight)();

  // Measure container width with ResizeObserver
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const measure = () => setContainerWidth(el.clientWidth);
    measure();

    const ro = new ResizeObserver(() => measure());
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Pre-compute aspect ratios
  const aspectRatios = useMemo(
    () => items.map((item) => {
      const ar = getAspectRatio(item);
      // Clamp extreme ratios to avoid degenerate rows
      return Math.max(0.3, Math.min(ar, 4));
    }),
    [items, getAspectRatio],
  );

  // Compute row layout
  const rows = useMemo(
    () => computeRows(aspectRatios, containerWidth, targetRowHeight, gap),
    [aspectRatios, containerWidth, targetRowHeight, gap],
  );

  return (
    <div ref={containerRef} data-testid="justified-grid">
      {rows.map((row) => {
        const rowItems = items.slice(row.startIdx, row.startIdx + row.count);
        const rowAspects = aspectRatios.slice(row.startIdx, row.startIdx + row.count);
        const isLastRow = row.startIdx + row.count >= items.length;
        // Only rows whose height was computed to exactly fill the container
        // width should use flex-grow. The last incomplete row must use fixed
        // widths so items keep their natural proportions and aren't stretched
        // (which causes photos/video previews to be cut off).
        const isFullRow = !isLastRow;

        return (
          <div
            key={row.startIdx}
            className="flex"
            style={{
              gap: `${gap}px`,
              marginBottom: `${gap}px`,
              height: `${row.height}px`,
            }}
          >
            {rowItems.map((item, i) => {
              const globalIdx = row.startIdx + i;
              const ar = rowAspects[i];
              // For full rows, use flex-grow proportional to aspect ratio.
              // For last incomplete row, use fixed width.
              const style: React.CSSProperties = isFullRow
                ? { flex: `${ar} 1 0%`, minWidth: 0 }
                : { width: `${ar * row.height}px`, flexShrink: 0 };

              return (
                <div
                  key={getKey(item)}
                  style={style}
                  className="overflow-hidden rounded"
                  data-testid="justified-grid-item"
                >
                  {renderItem(item, globalIdx)}
                </div>
              );
            })}
          </div>
        );
      })}
    </div>
  );
}
