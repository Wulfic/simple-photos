/**
 * The delta-sync cursor (#38): how far the local mirror has been brought
 * up to date with the server's photo change log.
 *
 * Kept in its own module because every read of it is a safety decision, not a
 * lookup. A cursor is a *claim* — "`db.photos` already contains every change up
 * to sequence N" — and acting on a false claim does not degrade gracefully. The
 * delta feed only ever names what changed *after* N, so if the mirror does not
 * actually hold everything up to N, the missing rows are never mentioned again
 * by any future response. The gallery is silently, permanently short, and no
 * amount of re-syncing repairs it. The full walk had no such failure mode: it
 * re-sent everything every time, so any local damage healed on the next pass.
 *
 * Two things buy that safety back:
 *
 *  1. **Co-location.** The cursor is a row in IndexedDB alongside the mirror
 *     (see `SyncStateRow`), so a storage eviction or `clearAllUserData()` takes
 *     both or neither. A localStorage cursor could outlive an evicted mirror.
 *  2. **The coherence guard below.** A cursor over an empty mirror is refused
 *     outright, which turns the one incoherent state that is cheap to detect
 *     into a full walk rather than an empty gallery.
 *
 * Neither is sufficient alone, and the cost of both is one indexed count per
 * sync pass.
 */
import { db } from "../../db";

/** Key of the single bookkeeping row holding the delta cursor. */
export const SYNC_CURSOR_KEY = "photoDeltaSeq";

/**
 * The sequence the mirror is current as of, or `null` to mean "unknown — do a
 * full walk".
 *
 * Returns `null` rather than throwing on any inconsistency. Every `null` here
 * costs one full walk; every wrongly-trusted number costs rows that never come
 * back. The asymmetry is the whole design.
 */
export async function readSyncCursor(): Promise<number | null> {
  try {
    const row = await db.syncState.get(SYNC_CURSOR_KEY);
    if (!row || !Number.isFinite(row.seq) || row.seq < 0) return null;

    // Coherence guard. A cursor asserts the mirror holds everything up to
    // `seq`; an empty mirror cannot possibly satisfy that unless the library
    // itself is empty — in which case a full walk is free anyway. This catches
    // the case co-location cannot: a partial wipe that empties `photos` while
    // `syncState` survives. `count()` is an index-only operation and does not
    // deserialize a single row.
    if (row.seq > 0 && (await db.photos.count()) === 0) {
      console.warn(
        "[sync] delta cursor present but the photo mirror is empty — " +
          "discarding the cursor and falling back to a full walk",
      );
      await clearSyncCursor();
      return null;
    }
    return row.seq;
  } catch (e) {
    // A cursor we cannot read is a cursor we must not trust.
    console.warn("[sync] failed to read the delta cursor; forcing a full walk", e);
    return null;
  }
}

/**
 * Advance the cursor after a pass has successfully applied everything it
 * received.
 *
 * Call this **only** once the mirror actually reflects `seq`. Writing it early —
 * before the reconcile commits, say — converts a transient failure into the
 * permanent gap described at the top of this file.
 */
export async function writeSyncCursor(seq: number): Promise<void> {
  if (!Number.isFinite(seq) || seq < 0) {
    console.warn("[sync] refusing to persist a non-finite delta cursor", seq);
    return;
  }
  try {
    await db.syncState.put({ key: SYNC_CURSOR_KEY, seq, updatedAt: Date.now() });
  } catch (e) {
    // Non-fatal: the mirror is correct, we just lost the shortcut. The next
    // pass reverts to a full walk, which is the pre-#38 behaviour.
    console.warn("[sync] failed to persist the delta cursor", e);
  }
}

/** Drop the cursor, forcing the next pass through a full, self-healing walk. */
export async function clearSyncCursor(): Promise<void> {
  try {
    await db.syncState.delete(SYNC_CURSOR_KEY);
  } catch (e) {
    console.warn("[sync] failed to clear the delta cursor", e);
  }
}
