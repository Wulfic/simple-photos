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
