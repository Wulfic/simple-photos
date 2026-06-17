/**
 * GallerySkeleton — loading placeholder that mirrors the justified photo grid
 * (JustifiedGrid). Renders a handful of flex rows whose tiles grow by a preset
 * aspect ratio, at the user's current target row height, so the skeleton has
 * the same rhythm/shape as the content it replaces (not generic boxes).
 *
 * Reused for every photo-grid surface: Gallery, Search results, Trash, and the
 * AlbumDetail photo tab. The shimmer + reduced-motion handling come from the
 * underlying <Skeleton> primitive.
 */
import { useThumbnailSizeStore } from "../../store/thumbnailSize";
import { Skeleton } from "../ui";

// Preset aspect-ratio rows (width/height per tile) chosen to look like a real
// justified layout — a mix of landscape, square and portrait tiles.
const ROWS: number[][] = [
  [1.5, 0.7, 1.3, 1.8],
  [1.2, 1.6, 0.8, 1.4, 1.1],
  [1.7, 1.0, 1.3],
  [0.9, 1.5, 1.2, 1.6, 0.8],
  [1.4, 1.1, 1.8, 1.0],
];

const GAP = 4;

export function GallerySkeleton({ rows = ROWS.length }: { rows?: number }) {
  const targetRowHeight = useThumbnailSizeStore((s) => s.targetRowHeight)();

  return (
    <div data-testid="gallery-skeleton" aria-hidden="true">
      {ROWS.slice(0, rows).map((aspects, r) => (
        <div
          key={r}
          className="flex"
          style={{ gap: GAP, marginBottom: GAP, height: targetRowHeight }}
        >
          {aspects.map((ar, i) => (
            <Skeleton
              key={i}
              variant="rect"
              style={{ flex: `${ar} 1 0%`, minWidth: 0 }}
            />
          ))}
        </div>
      ))}
    </div>
  );
}
