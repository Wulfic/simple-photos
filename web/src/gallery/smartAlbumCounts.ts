/**
 * Smart-album badge counts — the single place that decides what number sits on
 * an album card (#42).
 *
 * Three definitions of "how many photos are in this library" had silently
 * diverged: the server counted every eligible row, the web mirror dropped every
 * row still awaiting client-side encryption, and Android counted its whole Room
 * table. Measured on the live server, the web mirror was short by 2,494 of
 * 14,874 rows — about 17% — which is exactly the "Android says 10,211, web says
 * 7,822" the issue reported.
 *
 * The resolution: **the server summary is authoritative.** The mirror is only a
 * fallback for before that endpoint answers. Extracted out of `Albums.tsx` so
 * the arithmetic is unit-testable — inline in a page component it was not, which
 * is precisely how it drifted unnoticed.
 *
 * Every count here is a TILE count, not a row count: the album's filter is
 * applied first and burst frames are collapsed second, in that order, matching
 * `resolveAlbumPhotos`. Collapsing before filtering gives different (wrong)
 * answers for Favorites, where only some frames of a burst may be favourited.
 */
import type { CachedPhoto } from "../db";
import { collapseBursts } from "../utils/burstCollapse";
import { SMART_ALBUM_DEFS } from "./smartAlbums";
import type { PhotoSummary } from "../hooks/usePhotoSummary";

/** Tile counts backing the smart-album cards. */
export interface SmartAlbumCounts {
  all: number;
  recent: number;
  favorites: number;
  /** "Photos" is photo + gif, per `SMART_ALBUM_DEFS["smart-photos"]`. */
  photos: number;
  gifs: number;
  videos: number;
  audio: number;
}

/** The "Recently Added" cap, read from the album definitions rather than
 *  re-hardcoded, so this cannot drift from what the grid actually renders.
 *  The server mirrors it as `RECENT_ALBUM_LIMIT`. */
const RECENT_LIMIT = SMART_ALBUM_DEFS["smart-recent"].limit ?? 100;

/**
 * Counts from the server summary — the authoritative path.
 *
 * Returns `null` when the summary is absent OR predates this change: a server
 * on an older binary, or a summary persisted to localStorage before the
 * `smart_*` fields existed, answers without them. Reading through would paint
 * `undefined` into every badge, so callers fall back instead.
 */
export function countsFromSummary(summary: PhotoSummary | null): SmartAlbumCounts | null {
  if (!summary || typeof summary.smart_photos !== "number") return null;
  return {
    all: summary.collapsed_total,
    recent: summary.smart_recent,
    favorites: summary.smart_favorites,
    photos: summary.smart_photos,
    gifs: summary.smart_gifs,
    videos: summary.smart_videos,
    audio: summary.smart_audio,
  };
}

/**
 * Counts from the local IndexedDB mirror — the fallback path.
 *
 * Self-consistent (it collapses bursts exactly as the grids do) but still
 * structurally short by the pending-encryption backlog, because those rows are
 * not in the mirror at all. That is unavoidable here and is the reason this is
 * a fallback rather than the primary source.
 *
 * `photos` must already be secure-excluded by the caller.
 */
export function countsFromMirror(photos: CachedPhoto[] | undefined): SmartAlbumCounts | null {
  if (!photos || photos.length === 0) return null;
  const tiles = (subset: CachedPhoto[]) => collapseBursts(subset).length;
  const all = tiles(photos);
  return {
    all,
    recent: Math.min(all, RECENT_LIMIT),
    favorites: tiles(photos.filter((p) => !!p.isFavorite)),
    photos: tiles(photos.filter((p) => p.mediaType === "photo" || p.mediaType === "gif")),
    gifs: tiles(photos.filter((p) => p.mediaType === "gif")),
    videos: tiles(photos.filter((p) => p.mediaType === "video")),
    audio: tiles(photos.filter((p) => p.mediaType === "audio")),
  };
}

/**
 * Resolve the badge counts, server-first.
 *
 * Note the precedence: this used to be `local ?? summary`, which meant the
 * truncated mirror won as soon as it held a single row and the server summary
 * was never actually seen. That inversion is the fix.
 */
export function resolveSmartAlbumCounts(
  summary: PhotoSummary | null,
  mirror: CachedPhoto[] | undefined
): SmartAlbumCounts | null {
  return countsFromSummary(summary) ?? countsFromMirror(mirror);
}
