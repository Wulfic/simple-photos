/**
 * AlbumGridSkeleton — loading placeholder for the album card grid (Albums page
 * and the secure-gallery album list). Mirrors the real AlbumCard: a `card` with
 * a square cover, a title line and a smaller count line, laid out on the same
 * responsive 3→6 column grid.
 */
import { Skeleton } from "../ui";

export function AlbumGridSkeleton({ count = 12 }: { count?: number }) {
  return (
    <div
      className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 gap-3"
      data-testid="album-grid-skeleton"
      aria-hidden="true"
    >
      {Array.from({ length: count }).map((_, i) => (
        <div key={i} className="card p-2">
          <Skeleton variant="rect" height="auto" className="aspect-square w-full mb-1.5" />
          <Skeleton variant="text" width="70%" className="mb-1.5" />
          <Skeleton variant="text" width="40%" height="0.7em" />
        </div>
      ))}
    </div>
  );
}
