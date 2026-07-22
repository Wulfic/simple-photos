/**
 * Pure helpers for the cross-secure-album move picker (#31).
 *
 * A photo may live in at most ONE secure album (server-enforced), so pulling
 * media "from other secure albums" into the open album is a MOVE, not a copy.
 * These functions compute the source pool and resolve each pick to the move
 * request the server expects — kept free of `api`/React so they're unit-tested.
 */

/** Minimal shape needed to pick + move a secure item. */
export interface MovableSecureItem {
  id: string;
  gallery_id?: string | null;
}

/**
 * Items eligible to move INTO `currentGalleryId`: everything in the user's
 * other secure albums. Excludes the open album's own items and any item with no
 * owning gallery id (older rows we couldn't route a move for).
 */
export function otherSecureAlbumItems<T extends MovableSecureItem>(
  allItems: T[],
  currentGalleryId: string,
): T[] {
  return allItems.filter(
    (it) => !!it.gallery_id && it.gallery_id !== currentGalleryId,
  );
}

/** A single move: reassign `itemId` from `sourceGalleryId` to the target. */
export interface SecureMove {
  sourceGalleryId: string;
  itemId: string;
}

/**
 * Resolve a set of selected item ids to concrete move operations against the
 * source pool. Selections that aren't in the pool (or lack a source gallery)
 * are dropped — they can't be safely moved.
 */
export function resolveSecureMoves(
  pool: MovableSecureItem[],
  selectedItemIds: Iterable<string>,
): SecureMove[] {
  const byId = new Map(pool.map((it) => [it.id, it]));
  const moves: SecureMove[] = [];
  for (const id of selectedItemIds) {
    const it = byId.get(id);
    if (it?.gallery_id) moves.push({ sourceGalleryId: it.gallery_id, itemId: it.id });
  }
  return moves;
}

// ── Push direction (#43): move items OUT of the open album into another ──────
//
// The PULL picker above brings media in from other secure albums; the PUSH flow
// selects items in the album you're viewing and sends them elsewhere. The server
// operation is identical either way (`move_gallery_item` reassigns membership),
// so these helpers only differ in how the source/target are chosen.

/** An item that may be part of a burst stack. */
export interface BurstMovableItem extends MovableSecureItem {
  burst_id?: string | null;
}

/** A candidate target album for a push move. */
export interface SecureAlbumOption {
  id: string;
  name: string;
}

/**
 * Expand a selection of representative tile ids to every underlying item id,
 * pulling in all frames of any selected burst. The secure grid collapses a
 * burst to one tile, but a MOVE must carry every frame (mirrors secure-add,
 * which adds all frames) — otherwise a burst is split across two albums.
 */
export function expandSecureSelection<T extends BurstMovableItem>(
  allItems: T[],
  selectedIds: Iterable<string>,
): Set<string> {
  const framesByBurst = new Map<string, string[]>();
  for (const it of allItems) {
    if (it.burst_id) {
      const arr = framesByBurst.get(it.burst_id);
      if (arr) arr.push(it.id);
      else framesByBurst.set(it.burst_id, [it.id]);
    }
  }
  const byId = new Map(allItems.map((it) => [it.id, it]));
  const out = new Set<string>();
  for (const id of selectedIds) {
    out.add(id);
    const it = byId.get(id);
    if (it?.burst_id) {
      for (const fid of framesByBurst.get(it.burst_id) ?? []) out.add(fid);
    }
  }
  return out;
}

/**
 * Resolve a selection to concrete moves INTO `targetGalleryId`, dropping any
 * item already in the target (a no-op move) so the batch never issues a
 * pointless request or double-counts a success. Source gallery comes from each
 * item's own `gallery_id`, which is correct both for a real album (every item
 * shares it) and a synthetic smart view (items span several source albums).
 */
export function planSecureMovesToTarget(
  pool: MovableSecureItem[],
  selectedItemIds: Iterable<string>,
  targetGalleryId: string,
): SecureMove[] {
  return resolveSecureMoves(pool, selectedItemIds).filter(
    (mv) => mv.sourceGalleryId !== targetGalleryId,
  );
}

/**
 * Real secure albums a selection can be pushed INTO, excluding the album
 * currently open. For a synthetic smart view the open id matches nothing, so
 * every real album is offered — each selected item still routes from its own
 * source gallery, and same-source items are dropped by `planSecureMovesToTarget`.
 */
export function secureMoveTargets<T extends SecureAlbumOption>(
  albums: T[],
  currentGalleryId: string,
): T[] {
  return albums.filter((g) => g.id !== currentGalleryId);
}
