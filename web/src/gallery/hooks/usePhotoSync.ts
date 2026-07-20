/**
 * Hook that synchronises server-side encrypted photo records into IndexedDB.
 *
 * This is now only the React shell: lifecycle, the live query, the re-entrancy
 * guard and the backstop interval. The pass itself — skip / delta / full walk,
 * pagination, pruning, thumbnail decryption and dimension healing — lives in
 * `syncPass.ts`, where it can be tested by *operation count* rather than only
 * through a rendered hook (#38).
 */
import { useEffect, useRef, useState } from "react";
import { db, type CachedPhoto } from "../../db";
import { startThumbBackfill } from "../../db/thumbs";
import { runSyncPass } from "./syncPass";
import { useLiveQuery } from "dexie-react-hooks";

export interface PhotoSyncResult {
  /** Encrypted-mode photos from IndexedDB (live query, auto-updates).
   *  Returns `undefined` only until the Dexie query first resolves, then the
   *  cached array — the network sync refreshes it in the background. */
  encryptedPhotos: CachedPhoto[] | undefined;
  /** True during the initial sync only (not background polls). */
  loading: boolean;
  /** Trigger a server→IDB sync (idempotent, batched). */
  loadEncryptedPhotos: () => Promise<void>;
}

/** Re-sync interval in milliseconds.
 *
 * This is only a safety net: realtime changes already arrive over the SSE
 * stream (`/api/sync/events`), so the poll just catches anything a missed
 * event dropped. It used to be 2s, which — with no re-entrancy guard and a
 * full-library sync per tick — stacked overlapping syncs that re-paged the
 * entire photo table and re-downloaded thumbnails ~28×/s against the server
 * (see repo todo.md, "Idle Disk-Thrash Fix"). Five minutes is plenty for a
 * backstop poll; the guard below stops even this from stacking on slow links.
 *
 * Since #38 a tick on an unchanged library costs one small JSON request and
 * nothing else — see `syncPass.ts`. The interval is no longer load-bearing for
 * performance, only for staleness. */
const SYNC_INTERVAL_MS = 300_000;

export function usePhotoSync(): PhotoSyncResult {
  const [loading, setLoading] = useState(true);
  const [encryptedDataReady, setEncryptedDataReady] = useState(false);
  const syncIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  // Re-entrancy guard: the in-flight sync promise, or null when idle. A full
  // sync takes far longer than the interval on a large library, so without this
  // the interval tick (and any explicit refresh) would stack overlapping runs —
  // each seeing stale Dexie state and re-downloading thumbnails the others
  // haven't persisted yet. Concurrent callers coalesce onto the same run.
  const syncInFlightRef = useRef<Promise<void> | null>(null);

  // Live query — auto-updates when IDB changes
  const rawEncryptedPhotos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray(),
  );

  // Show whatever IndexedDB already holds **immediately**, and treat the network
  // sync as a background refresh rather than a precondition for display. This is
  // safe against "flash of the previous user's photos" because IDB is wiped by
  // `clearAllUserData()` on every login/logout/401 (see db/index.ts), so any
  // cached rows always belong to the current session. Previously this returned
  // `undefined` until `encryptedDataReady` flipped true — which only happened
  // after a full network re-sync completed — so Albums/Gallery showed a spinner
  // on *every* open even though the persisted data was right there. `undefined`
  // now means only "the Dexie query hasn't resolved yet" (near-instant).
  const encryptedPhotos = rawEncryptedPhotos;

  // ── Legacy thumbnail migration ────────────────────────────────────────
  // Move any thumbnail bytes still sitting inline on photo rows into the
  // `thumbs` table (see db/thumbs.ts). Runs once per session, in the
  // background, and is a cheap no-op after the first pass has drained.
  useEffect(() => {
    startThumbBackfill();
  }, []);

  // ── Periodic re-sync ──────────────────────────────────────────────────
  useEffect(() => {
    if (!encryptedDataReady) return;
    syncIntervalRef.current = setInterval(() => {
      loadEncryptedPhotos().catch(() => {});
    }, SYNC_INTERVAL_MS);
    return () => {
      if (syncIntervalRef.current) {
        clearInterval(syncIntervalRef.current);
        syncIntervalRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [encryptedDataReady]);

  // ── Core sync logic ───────────────────────────────────────────────────

  /** Trigger a server→IDB sync. Re-entrant callers (interval tick + explicit
   *  refresh) coalesce onto the single in-flight run instead of stacking. */
  function loadEncryptedPhotos(): Promise<void> {
    if (syncInFlightRef.current) return syncInFlightRef.current;
    const run = syncEncryptedPhotos().finally(() => {
      syncInFlightRef.current = null;
    });
    syncInFlightRef.current = run;
    return run;
  }

  async function syncEncryptedPhotos() {
    if (!encryptedDataReady) setLoading(true);
    try {
      // The pass itself lives in `syncPass.ts`: it picks between skipping
      // outright (nothing changed), a delta, and the full self-healing walk.
      // `onDataReady` fires once the mirror is safe to present — after any
      // pruning, before the reconcile — which is where this hook used to flip
      // the flag inline.
      await runSyncPass({ onDataReady: () => setEncryptedDataReady(true) });
    } catch (err: unknown) {
      // Propagate to caller — useGalleryData will set the error
      throw err;
    } finally {
      setLoading(false);
    }
  }

  return { encryptedPhotos, loading, loadEncryptedPhotos };
}
