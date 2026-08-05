/**
 * The persisted "Takeout albums are fully materialized" latch.
 *
 * Reconstruction is idempotent but not free — it fetches the server's
 * source-album mapping and walks every album against the local mirror. The latch
 * used to be a `useRef`, so it reset on every page mount: each visit to the
 * Albums page re-ran the whole pass, and if the mirror was still syncing, it
 * re-uploaded manifests for albums it had already built.
 *
 * What's stored is the mirror size at the moment a pass reported nothing left to
 * do — not a bare "done" flag. That keeps the steady state free while staying
 * self-healing: the instant the mirror actually grows, the recorded size no
 * longer matches and reconstruction runs again to pick up what arrived. A latch
 * that could never re-open would mean permanently incomplete albums, which is
 * strictly worse than a redundant pass.
 *
 * Scoped per (server, user): a shared browser must never let one account's latch
 * suppress another's reconstruction.
 */

const PREFIX = "sp:takeout-materialized";

function key(scope: string): string {
  return `${PREFIX}:${scope}`;
}

/**
 * The mirror size when reconstruction last settled for `scope`, or -1 if it
 * never has (so any real count differs and a pass runs).
 */
export function getMaterializedAt(scope: string): number {
  try {
    const raw = localStorage.getItem(key(scope));
    if (raw === null) return -1;
    const n = Number.parseInt(raw, 10);
    return Number.isFinite(n) ? n : -1;
  } catch {
    // Storage disabled/full (private mode). Reconstruction just re-runs — it's
    // idempotent, so the only cost is the work itself.
    return -1;
  }
}

/** Record that reconstruction settled for `scope` at a mirror of `photoCount`. */
export function setMaterializedAt(scope: string, photoCount: number): void {
  try {
    localStorage.setItem(key(scope), String(photoCount));
  } catch {
    // See getMaterializedAt — non-fatal by design.
  }
}

/** Forget the latch for `scope` (e.g. on logout). */
export function clearMaterializedAt(scope: string): void {
  try {
    localStorage.removeItem(key(scope));
  } catch {
    /* non-fatal */
  }
}

/**
 * Whether reconstruction should run: it has never settled, or the mirror has
 * changed size since it did.
 */
export function shouldReconstruct(scope: string, photoCount: number): boolean {
  return photoCount > 0 && getMaterializedAt(scope) !== photoCount;
}
