/**
 * Smart album definitions — the synthetic albums (Recently Added, Favorites,
 * Photos, GIFs, Videos, Audio) computed from the local encrypted photo cache
 * rather than a user-created album manifest.
 *
 * Extracted from SmartAlbumView so the unified `useAlbumPhotos` hook and the
 * view share one source of truth for what each smart album contains, its
 * ordering, and its cap. `AlbumDetail` uses `isSmartAlbum` to route.
 */
import type { CachedPhoto } from "../db";

export type SmartAlbumDef = {
  label: string;
  filterEncrypted: (p: CachedPhoto) => boolean;
  /** When set, override the default takenAt-desc ordering. "addedAt" sorts by
   *  library import order (falls back to takenAt when addedAt is absent). */
  sortBy?: "addedAt";
  /** When set, cap the album to the N most-recent items after sorting. */
  limit?: number;
};

export const SMART_ALBUM_DEFS: Record<string, SmartAlbumDef> = {
  "smart-recent": {
    label: "Recently Added",
    filterEncrypted: () => true,
    sortBy: "addedAt",
    limit: 100,
  },
  "smart-favorites": {
    label: "Favorites",
    filterEncrypted: (p) => !!p.isFavorite,
  },
  "smart-photos": {
    label: "Photos",
    filterEncrypted: (p) => p.mediaType === "photo" || p.mediaType === "gif",
  },
  "smart-gifs": {
    label: "GIFs",
    filterEncrypted: (p) => p.mediaType === "gif",
  },
  "smart-videos": {
    label: "Videos",
    filterEncrypted: (p) => p.mediaType === "video",
  },
  "smart-audio": {
    label: "Audio",
    filterEncrypted: (p) => p.mediaType === "audio",
  },
};

export function isSmartAlbum(id: string | undefined): id is string {
  return !!id && id in SMART_ALBUM_DEFS;
}
