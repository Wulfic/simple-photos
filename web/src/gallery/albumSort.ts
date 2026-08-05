/**
 * Album sort (#52) — a user-selectable ordering for the album-detail view.
 *
 * Ordering used to be hardcoded: `takenAt` desc everywhere, with the single
 * `sortBy: "addedAt"` special case for "Recently Added". This adds a Date/Name
 * sort control to the album header. The choice is per-album and applied
 * uniformly in `useAlbumPhotos.resolveAlbumPhotos` (never special-cased per
 * view), **after** burst collapse so a burst sorts by its representative frame.
 *
 * The sort is deliberately *optional*: when a user has made no choice, the
 * album keeps its intrinsic order (which is what preserves "Recently Added"'s
 * add-order). Only an explicit selection triggers a post-sort.
 */
import type { PhotoWithBurstCount } from "../utils/burstCollapse";

export type SortField = "date" | "name";
export type SortDir = "asc" | "desc";
export interface AlbumSort {
  field: SortField;
  dir: SortDir;
}

/** The historical ordering before #52: capture date, newest first. Used as the
 *  control's visual default when the user has not chosen a sort. */
export const DEFAULT_ALBUM_SORT: AlbumSort = { field: "date", dir: "desc" };

/** The direction a field starts in when you first switch to it: dates
 *  newest-first, names A→Z — what a first click on each is expected to do. */
export function defaultDirFor(field: SortField): SortDir {
  return field === "name" ? "asc" : "desc";
}

// `numeric: true` so "IMG_2" precedes "IMG_10" instead of sorting lexically;
// `sensitivity: "base"` makes it case- and accent-insensitive, matching how a
// file browser orders names.
const collator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

/**
 * Total order over resolved album photos. Always breaks ties on `blobId` so the
 * result is deterministic — two photos sharing a capture time or filename never
 * swap places between renders, which is the stability `JustifiedGrid` needs to
 * keep the scroll position from jumping.
 */
export function compareAlbumPhotos(
  a: PhotoWithBurstCount,
  b: PhotoWithBurstCount,
  sort: AlbumSort
): number {
  let cmp: number;
  if (sort.field === "name") {
    cmp = collator.compare(a.filename ?? "", b.filename ?? "");
  } else {
    // A missing capture time (0) sorts as the oldest — last under desc, first
    // under asc — rather than throwing the row to a random position.
    cmp = (a.takenAt ?? 0) - (b.takenAt ?? 0);
  }
  if (cmp === 0) {
    cmp = a.blobId < b.blobId ? -1 : a.blobId > b.blobId ? 1 : 0;
  }
  return sort.dir === "asc" ? cmp : -cmp;
}

/** Return a new, sorted array — never mutates the input list. */
export function sortAlbumPhotos<T extends PhotoWithBurstCount>(
  photos: T[],
  sort: AlbumSort
): T[] {
  return [...photos].sort((a, b) => compareAlbumPhotos(a, b, sort));
}

// ── Persistence (per album, localStorage) ──────────────────────────────────
// Keyed by album id, matching the per-view `useScrollMemory` precedent. A
// regular album's manifest id and a smart album's synthetic id are both stable,
// so the choice survives navigation and restarts.

const keyFor = (albumId: string) => `albumSort:${albumId}`;

/** The user's chosen sort for this album, or `null` if they have not set one
 *  (in which case the album keeps its intrinsic order). Tolerates absent or
 *  corrupt storage — a bad value reads as "no choice", never throws. */
export function readAlbumSort(albumId: string): AlbumSort | null {
  try {
    const raw = localStorage.getItem(keyFor(albumId));
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    return isAlbumSort(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

export function writeAlbumSort(albumId: string, sort: AlbumSort): void {
  try {
    localStorage.setItem(keyFor(albumId), JSON.stringify(sort));
  } catch {
    // Storage full or blocked (private mode): the choice still applies this
    // session via component state; only its persistence is lost.
  }
}

export function isAlbumSort(v: unknown): v is AlbumSort {
  if (!v || typeof v !== "object") return false;
  const s = v as Partial<AlbumSort>;
  return (
    (s.field === "date" || s.field === "name") &&
    (s.dir === "asc" || s.dir === "desc")
  );
}
