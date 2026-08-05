/**
 * Pure helpers for the album "Add photos" picker.
 *
 * The regular-album picker is fed the entire local mirror (a real library is
 * ~7000 photos), so it needs a client-side filter to be usable rather than an
 * endless flat scroll — #27 ("doesn't present our photo selector ... but does
 * say all ~7000 photos are there"). Kept pure (no React) so the narrowing logic
 * is unit-testable in isolation, matching the resolveAlbumPhotos pattern.
 */
import type { CachedPhoto } from "../db";

/**
 * Narrow a picker's photo list by a case-insensitive filename substring. An
 * empty/whitespace query returns the list unchanged (same reference). Photos
 * with no filename never match a non-empty query.
 */
export function filterPickerPhotos(
  photos: CachedPhoto[],
  query: string,
): CachedPhoto[] {
  const q = query.trim().toLowerCase();
  if (!q) return photos;
  return photos.filter((p) => p.filename?.toLowerCase().includes(q));
}
