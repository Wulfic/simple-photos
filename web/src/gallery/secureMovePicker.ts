/**
 * Pure helpers for the cross-secure-album pickers (#31 pull, #43 push).
 *
 * **The one-secure-album rule is gone (Z1).** A photo may now live in several
 * secure albums at once, sharing a single encrypted clone server-side. What
 * survives is only "at most once per *album*".
 *
 * That splits what used to be one operation into two, and they are not
 * interchangeable:
 *
 *  - **MOVE** (`moveItem`) — reassigns the membership row. The photo leaves the
 *    source album. Still what the #31 pull picker wants: "bring these here".
 *  - **ADD** (`addItem`) — creates an additional membership row against the same
 *    clone. The photo is in both albums. This is what the "+" button in an album
 *    header means everywhere else in the app, and it is what the #43 push flow
 *    was silently getting wrong: it offered a "+"-shaped affordance and then
 *    moved, so filing a photo into a second album quietly removed it from the
 *    first.
 *
 * Kept free of `api`/React so they're unit-tested.
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
 * Real secure albums a selection can be pushed INTO, excluding the album
 * currently open. For a synthetic smart view the open id matches nothing, so
 * every real album is offered — each selected item still routes from its own
 * source gallery.
 *
 * (`planSecureMovesToTarget` used to live here and filtered the same-source
 * items out of a *move* batch. Z1 replaced the push with adds, leaving it
 * exported, tested and called by nothing — the same half-wiring shape `56f995c`
 * shipped one level up. Deleted rather than kept beside its replacement, as
 * Android's twin `planMovesToTarget` was in Z1e; `planSecureAddsToTarget` is
 * the live path.)
 */
export function secureMoveTargets<T extends SecureAlbumOption>(
  albums: T[],
  currentGalleryId: string,
): T[] {
  return albums.filter((g) => g.id !== currentGalleryId);
}

// ── Add direction (Z1): file items into another album, keeping them here ─────

/** An item that can be added to another secure album by its clone blob id. */
export interface AddableSecureItem extends BurstMovableItem {
  /** The clone blob the server keys an adoption on. */
  blob_id?: string | null;
}

/** A single add: give `blobId` an additional membership in the target album. */
export interface SecureAdd {
  itemId: string;
  blobId: string;
}

/**
 * Resolve a selection to concrete ADDs into `targetGalleryId`.
 *
 * Two deliberate omissions:
 *
 * 1. **No "is it already in the target" filter.** The move planner has one,
 *    because a move's source is knowable from the item's own `gallery_id`. An
 *    add's answer lives in a *different* album's membership rows, which the
 *    per-album feed this runs against simply does not carry. The server already
 *    answers it authoritatively with a 409, so the caller treats that as
 *    "already there" rather than as a failure. Deriving it here would be a
 *    second derivation of membership — the exact drift this repo has recorded
 *    nine times — and it would be the *wrong* one, since it would be guessing
 *    from a feed that cannot see the target.
 * 2. **`blob_id`, not `id`.** An add is keyed on the clone blob (the server
 *    matches it to find the donor membership and adopt it); the item id is
 *    carried only so a caller can report per-item outcomes.
 */
export function planSecureAddsToTarget(
  pool: AddableSecureItem[],
  selectedItemIds: Iterable<string>,
): SecureAdd[] {
  const byId = new Map(pool.map((it) => [it.id, it]));
  const out: SecureAdd[] = [];
  const seenBlobs = new Set<string>();
  for (const id of selectedItemIds) {
    const it = byId.get(id);
    if (!it?.blob_id) continue;
    // One add per clone: two selected burst frames sharing a clone would
    // otherwise issue two requests, the second guaranteed to 409.
    if (seenBlobs.has(it.blob_id)) continue;
    seenBlobs.add(it.blob_id);
    out.push({ itemId: it.id, blobId: it.blob_id });
  }
  return out;
}
