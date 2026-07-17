/**
 * Secure smart album definitions — the built-in synthetic albums that appear in
 * the Secure Albums section (Secure Gallery / Photos / GIFs / Videos / Audio),
 * derived entirely from the aggregate `/galleries/secure/items` feed. Nothing is
 * stored server-side; membership is a pure function of each item's `media_type`.
 *
 * These deliberately use a `secure-smart-*` namespace so they never collide with
 * the main gallery's `smart-*` ids (wired into `useAlbumPhotos`/`AlbumDetail`
 * routing and the secure-add picker).
 */
import type { SecureGalleryItem } from "../api/galleries";

export type SecureSmartAlbumDef = {
  id: string;
  label: string;
  filter: (item: SecureGalleryItem) => boolean;
};

/**
 * Ordered so tiles render Secure Gallery → Photos → GIFs → Videos → Audio.
 *
 * `secure-smart-photos` mirrors the main gallery's `smart-photos`: it includes
 * GIFs, and a NULL `media_type` (backup servers where the clone `photos` row is
 * absent) falls back here so nothing silently vanishes from the union view.
 */
export const SECURE_SMART_ALBUM_DEFS: SecureSmartAlbumDef[] = [
  { id: "secure-smart-all", label: "Secure Gallery", filter: () => true },
  {
    id: "secure-smart-photos",
    label: "Photos",
    filter: (i) =>
      i.media_type === "photo" || i.media_type === "gif" || i.media_type == null,
  },
  { id: "secure-smart-gifs", label: "GIFs", filter: (i) => i.media_type === "gif" },
  {
    id: "secure-smart-videos",
    label: "Videos",
    filter: (i) => i.media_type === "video",
  },
  { id: "secure-smart-audio", label: "Audio", filter: (i) => i.media_type === "audio" },
];

const SECURE_SMART_IDS = new Set(SECURE_SMART_ALBUM_DEFS.map((d) => d.id));

/** True when `id` names one of the built-in secure smart albums. */
export function isSecureSmartAlbum(id: string | undefined | null): boolean {
  return !!id && SECURE_SMART_IDS.has(id);
}

/** Look up a smart-album definition by id (undefined for non-smart ids). */
export function secureSmartAlbumDef(id: string): SecureSmartAlbumDef | undefined {
  return SECURE_SMART_ALBUM_DEFS.find((d) => d.id === id);
}

export type SecureSmartAlbum = {
  id: string;
  label: string;
  count: number;
  /** Newest matching item — its thumbnail becomes the album cover. */
  coverItem: SecureGalleryItem;
};

/**
 * Compute the visible secure smart albums from the aggregate item feed.
 *
 * Only albums with at least one matching item are returned (empty types never
 * render a tile). `items` is assumed to already be in `added_at DESC` order (the
 * server contract), so the first match for each def is the newest item and is
 * used as the cover. `count` is the raw membership count (bursts NOT collapsed),
 * matching how real secure-album cards show `item_count`.
 */
export function computeSecureSmartAlbums(
  items: SecureGalleryItem[]
): SecureSmartAlbum[] {
  const result: SecureSmartAlbum[] = [];
  for (const def of SECURE_SMART_ALBUM_DEFS) {
    let count = 0;
    let coverItem: SecureGalleryItem | undefined;
    for (const it of items) {
      if (!def.filter(it)) continue;
      count++;
      if (!coverItem) coverItem = it; // first match = newest (added_at DESC)
    }
    if (count > 0 && coverItem) {
      result.push({ id: def.id, label: def.label, count, coverItem });
    }
  }
  return result;
}

/** Filter the aggregate feed down to a single smart album's members. */
export function filterSecureSmartAlbum(
  items: SecureGalleryItem[],
  id: string
): SecureGalleryItem[] {
  const def = secureSmartAlbumDef(id);
  if (!def) return [];
  return items.filter(def.filter);
}
