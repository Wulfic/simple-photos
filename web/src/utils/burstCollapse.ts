/**
 * Burst-stack collapse.
 *
 * Burst stacks are shown as a single representative tile in every photo grid
 * (main gallery, smart albums, search). Previously each surface re-implemented
 * this grouping inline (Gallery.tsx, SmartAlbumView.tsx, Search.tsx) with
 * subtly different ordering — this is the single canonical implementation.
 *
 * Groups by `burstId`, keeping the FIRST frame encountered as the stack
 * representative, so the output preserves the caller's input order (pass a
 * pre-sorted list). The representative is stamped with `_burstCount` (total
 * frames in the group) to drive the stack badge. Non-burst photos pass through
 * untouched.
 *
 * Objects are COPIED, never mutated: callers share them with the Dexie
 * live-query cache, and a stale `_burstCount` stamped onto a cached object kept
 * showing a burst badge after the group shrank (see Gallery.tsx history).
 */
import type { CachedPhoto } from "../db";

export type PhotoWithBurstCount = CachedPhoto & { _burstCount?: number };

export function collapseBursts(
  photos: readonly CachedPhoto[]
): PhotoWithBurstCount[] {
  // First pass: total frames per burst group.
  const counts = new Map<string, number>();
  for (const p of photos) {
    if (p.burstId) counts.set(p.burstId, (counts.get(p.burstId) ?? 0) + 1);
  }

  // Second pass: emit each non-burst photo, and the first frame of each burst
  // group (stamped with its count), preserving input order.
  const seen = new Set<string>();
  const out: PhotoWithBurstCount[] = [];
  for (const p of photos) {
    if (!p.burstId) {
      out.push(p);
      continue;
    }
    if (seen.has(p.burstId)) continue;
    seen.add(p.burstId);
    out.push({ ...p, _burstCount: counts.get(p.burstId) });
  }
  return out;
}
