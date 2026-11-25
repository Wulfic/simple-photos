/**
 * Removing items from a secure album: plan → ask → execute → report.
 *
 * Two places offer it — the secure grid (one tile's trash button, or a
 * selection) and the secure viewer's overflow menu (F4). Both MUST describe and
 * perform the same operation, so both go through this module. A second,
 * separately derived copy of this logic is precisely how Z1d happened: the
 * toast was computed apart from the prompt and promised "returned to your
 * gallery" for photos that stayed secured.
 *
 * The pipeline, in one place:
 *  1. {@link planSecureRemoval} expands burst tiles to whole stacks (S2), routes
 *     each item to its OWNING album (a smart view's id is synthetic), counts the
 *     other secure albums holding each one, and builds the prompt from exactly
 *     that expanded set.
 *  2. {@link executeSecureRemoval} removes what the plan names — nothing else —
 *     isolating failures per item.
 *  3. {@link secureRemovalOutcome} reports from the plan's own counts.
 *
 * Kept free of `api`/React/Dexie (the side effects are injected) so all three
 * are unit-tested without a DOM. Android's twin is `GalleryDetailView`'s
 * `pendingRemoval` block over `SecureMovePlan.expandForRemoval`.
 */
import {
  otherSecureAlbumCount,
  secureRemovalPrompt,
  secureRemovalToast,
  type SecureRemovalVerdict,
} from "./albumRemoval";
import { expandBurstStacks, type BurstMovableItem } from "./secureMovePicker";
import { isSecureSmartAlbum } from "./secureSmartAlbums";

/** The minimal shape a secure item needs to be planned for removal. */
export interface RemovableSecureItem extends BurstMovableItem {
  blob_id: string;
  /** Set only by the aggregate feed. */
  gallery_name?: string | null;
  /** Every secure album holding the photo (Z1). Absent/empty = UNKNOWN. */
  galleries?: ReadonlyArray<{ id: string; name: string }> | null;
}

/** The album the user is looking at — real, or a synthetic smart view. */
export interface OpenSecureAlbum {
  id: string;
  /** Known for a real album opened from the grid; the viewer may not have it. */
  name?: string;
}

export interface SecureRemovalTarget<T> {
  item: T;
  /** The real album this membership row lives in; undefined = cannot route. */
  owningGalleryId: string | undefined;
}

export interface SecureRemovalPlan<T> {
  /** Everything the removal acts on — bursts already expanded. */
  targets: SecureRemovalTarget<T>[];
  /** Per target: how many OTHER secure albums hold it; `undefined` = unknown. */
  otherAlbums: (number | undefined)[];
  verdict: SecureRemovalVerdict;
}

export interface SecureRemovalResult {
  /** Per removed item: its other-album count, as the prompt described it. */
  removedCounts: number[];
  failed: number;
}

export interface SecureRemovalDeps {
  removeItem: (galleryId: string, itemId: string) => Promise<unknown>;
  /** Drop the local IDB clone row created at add time. Best effort. */
  dropCacheRow: (blobId: string) => Promise<unknown>;
}

/**
 * Plan the removal of `tiles` (what the user tapped) from `open`, where `pool`
 * is every item the tiles were drawn from, UN-collapsed — the burst siblings
 * are found there.
 *
 * Returns `null` for an empty tap list: there is nothing to ask about, and a
 * plan for nothing would render the "unknown membership" refusal.
 */
export function planSecureRemoval<T extends RemovableSecureItem>(
  pool: ReadonlyArray<T>,
  tiles: ReadonlyArray<T>,
  open: OpenSecureAlbum,
): SecureRemovalPlan<T> | null {
  if (tiles.length === 0) return null;
  // In a smart view the open "album" is synthetic — route each removal to the
  // item's REAL owning album. `gallery_id` is present on both feeds.
  const smartView = isSecureSmartAlbum(open.id);
  const targets = expandBurstStacks(pool, tiles).map((item) => ({
    item,
    owningGalleryId: smartView ? (item.gallery_id ?? undefined) : open.id,
  }));
  // No owning album means we cannot even name the membership set — unknown,
  // which the prompt turns into a refusal rather than a guess.
  const otherAlbums = targets.map(({ item, owningGalleryId }) =>
    owningGalleryId ? otherSecureAlbumCount(item.galleries, owningGalleryId) : undefined,
  );
  // The album NAME must come from the owning album, not the open view: in a
  // smart view the open name is "Videos", and naming that as the thing being
  // removed from is wrong in the one dialog whose job is accuracy. A batch
  // spanning several albums names none rather than the wrong one.
  const owningNames = new Set(
    targets.map(({ item, owningGalleryId }) =>
      smartView
        ? (item.gallery_name ?? membershipName(item, owningGalleryId))
        : (open.name ?? membershipName(item, owningGalleryId)),
    ),
  );
  const owningName = owningNames.size === 1 ? [...owningNames][0] : undefined;
  return { targets, otherAlbums, verdict: secureRemovalPrompt(otherAlbums, owningName) };
}

function membershipName(item: RemovableSecureItem, galleryId: string | undefined): string | undefined {
  if (!galleryId) return undefined;
  return item.galleries?.find((g) => g.id === galleryId)?.name;
}

/**
 * Perform the removal a CONFIRMED plan describes. Each item is isolated, so one
 * failure never aborts the rest; a target the plan could not route is counted
 * as failed, never guessed at.
 *
 * Whether a photo returns to the regular gallery is the server's call — it does
 * once its LAST secure membership is gone — and the plan's counts already say
 * which way each one goes.
 */
export async function executeSecureRemoval<T extends RemovableSecureItem>(
  plan: SecureRemovalPlan<T>,
  deps: SecureRemovalDeps,
): Promise<SecureRemovalResult> {
  const removedCounts: number[] = [];
  let failed = 0;
  for (let i = 0; i < plan.targets.length; i++) {
    const { item, owningGalleryId } = plan.targets[i];
    const others = plan.otherAlbums[i];
    if (!owningGalleryId || others === undefined) {
      // Unreachable behind a `confirm` verdict; counted, never guessed.
      console.error("[secureRemoval] removal target without a known owning album", item.id);
      failed++;
      continue;
    }
    try {
      await deps.removeItem(owningGalleryId, item.id);
      removedCounts.push(others);
    } catch (err) {
      console.error("[secureRemoval] remove item failed", item.id, err);
      failed++;
      continue;
    }
    // The membership is gone server-side; a stale cache row only lasts until
    // the next reload, so this failure is logged but is not a failed removal.
    try {
      await deps.dropCacheRow(item.blob_id);
    } catch (err) {
      console.warn("[secureRemoval] could not drop local cache row", item.blob_id, err);
    }
  }
  return { removedCounts, failed };
}

/** A removal's user-facing outcome. */
export interface SecureFlash {
  success: string;
  error: string;
}

/**
 * What to tell the user afterwards — from the counts the prompt was built on,
 * so the toast cannot claim an outcome the dialog did not describe (Z1d).
 */
export function secureRemovalOutcome(result: SecureRemovalResult): SecureFlash {
  return {
    success: result.removedCounts.length > 0 ? secureRemovalToast(result.removedCounts) : "",
    error:
      result.failed > 0
        ? `${result.failed} item${result.failed !== 1 ? "s" : ""} couldn't be removed.`
        : "",
  };
}

/**
 * The viewer closes on a confirmed removal (Android parity), so its outcome has
 * to cross the route into the grid: it travels as `location.state.secureFlash`.
 * Route state is `unknown` to the reader — anything malformed reads as no flash
 * rather than as a toast built from whatever happened to be there.
 */
export function readSecureFlash(state: unknown): SecureFlash | null {
  if (typeof state !== "object" || state === null || !("secureFlash" in state)) return null;
  const flash: unknown = state.secureFlash;
  if (typeof flash !== "object" || flash === null) return null;
  const success = "success" in flash ? flash.success : undefined;
  const error = "error" in flash ? flash.error : undefined;
  if (typeof success !== "string" || typeof error !== "string") return null;
  if (!success && !error) return null;
  return { success, error };
}
