/**
 * Hook that synchronises server-side encrypted photo records into IndexedDB.
 *
 * Handles cursor-based pagination, stale-entry cleanup, thumbnail decryption,
 * dimension healing, and periodic background re-sync.
 */
import { useEffect, useRef, useState } from "react";
import { api } from "../../api/client";
import { decrypt } from "../../crypto/crypto";
import { decryptBlobMetadata } from "../../crypto/blobEnvelope";
import {
  db,
  type CachedPhoto,
  type CachedThumb,
  mediaTypeFromMime,
} from "../../db";
import { startThumbBackfill } from "../../db/thumbs";
import { base64ToArrayBuffer } from "../../utils/media";
import { fetchAllPages } from "../../utils/gallery";
import { reconcileSyncedPhotos, RECONCILE_CHUNK, type SyncRecord } from "./syncReconcile";
import { useLiveQuery } from "dexie-react-hooks";
import type { PhotoPayload, ThumbnailPayload } from "../../types/media";

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
 * backstop poll; the guard below stops even this from stacking on slow links. */
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
      // Phase 1: Fetch metadata via encrypted-sync endpoint.
      const allSyncPhotos: SyncRecord[] = [];
      let cursor: string | undefined;
      do {
        const res = await api.photos.encryptedSync({ after: cursor, limit: 500 });
        allSyncPhotos.push(...res.photos);
        cursor = res.next_cursor ?? undefined;
      } while (cursor);

      const serverPhotoIds = new Set<string>();
      const serverBlobIds = new Set<string>();
      for (const p of allSyncPhotos) {
        serverPhotoIds.add(p.id);
        if (p.encrypted_blob_id) serverBlobIds.add(p.encrypted_blob_id);
      }

      // Phase 2: Include directly-uploaded encrypted blobs.
      const allBlobMedia = [
        ...(await fetchAllPages("photo")),
        ...(await fetchAllPages("gif")),
        ...(await fetchAllPages("video")),
        ...(await fetchAllPages("audio")),
      ];
      for (const blob of allBlobMedia) serverBlobIds.add(blob.id);

      // Remove stale IDB entries
      const currentCached = await db.photos.toArray();
      const staleIds = new Set(
        currentCached
          .filter((p) => {
            if (p.serverPhotoId) return !serverPhotoIds.has(p.serverPhotoId);
            const underlyingId = p.storageBlobId || p.blobId;
            return !serverBlobIds.has(underlyingId) && !serverPhotoIds.has(underlyingId);
          })
          .map((p) => p.blobId),
      );
      if (staleIds.size > 0) await db.photos.bulkDelete([...staleIds]);

      setEncryptedDataReady(true);

      // Derived, not re-read: the previous version issued a second full
      // `toArray()` here purely to see the effect of the delete it had just
      // performed. On a large library that is a second full deserialization of
      // the mirror for information already in hand.
      const survivingCached = currentCached.filter((p) => !staleIds.has(p.blobId));

      // Phase 3: Populate IDB from sync records.
      //
      // Batched, chunked and staged — see `syncReconcile.ts`. This was a
      // per-photo loop with an awaited IndexedDB read *and* write inside it, so
      // a 10k library meant 10k+ serialized transactions on the main thread
      // every pass. That is the bulk of "photo libraries are slow" (#38).
      await reconcileSyncedPhotos(allSyncPhotos, survivingCached);

      // Phase 4: Handle directly-uploaded encrypted blobs not in photos table.
      const syncedBlobIds = new Set(
        allSyncPhotos.map((p) => p.encrypted_blob_id).filter((id): id is string => !!id),
      );
      // Membership is answered by ONE indexed key scan rather than a
      // `db.photos.get()` per blob as this previously did. `primaryKeys()` reads
      // the index without deserializing rows, and — unlike the pre-reconcile
      // snapshot — it includes anything Phase 3 just inserted. That matters:
      // `photos.id` and blob ids share a namespace on client-upload paths (hence
      // the `id NOT IN (SELECT blob_id ...)` guard in the sync query), so a
      // stale snapshot here could re-insert a row Phase 3 had just written.
      const cachedBlobIds = new Set((await db.photos.toCollection().primaryKeys()) as string[]);
      const unsyncedBlobs = allBlobMedia.filter(
        (b) => !syncedBlobIds.has(b.id) && !cachedBlobIds.has(b.id),
      );

      const directPhotos: CachedPhoto[] = [];
      const directThumbs: CachedThumb[] = [];

      /** Commit what Phase 4 has accumulated so far, then clear the buffers. */
      const flushDirect = async () => {
        if (directPhotos.length === 0 && directThumbs.length === 0) return;
        const photoRows = directPhotos.splice(0);
        const thumbRows = directThumbs.splice(0);
        try {
          await db.transaction("rw", db.photos, db.thumbs, async () => {
            if (photoRows.length > 0) await db.photos.bulkPut(photoRows);
            if (thumbRows.length > 0) await db.thumbs.bulkPut(thumbRows);
          });
        } catch (e) {
          // One bad chunk must not abandon the rest; the next pass retries it.
          console.warn("[sync] direct-blob chunk failed to commit", e);
        }
      };

      for (const blob of unsyncedBlobs) {
        try {
          const encrypted = await api.blobs.download(blob.id);
          // Only metadata is needed here — decryptBlobMetadata avoids
          // reconstructing media bytes (and, for v2, avoids decrypting every
          // chunk frame just to read fields). Handles both blob formats.
          const payload = await decryptBlobMetadata<PhotoPayload>(encrypted);

          let thumbnailData: ArrayBuffer | undefined;
          let unsyncedThumbMime: string | undefined;
          if (payload.thumbnail_blob_id) {
            try {
              const thumbEnc = await api.blobs.download(payload.thumbnail_blob_id);
              const thumbDec = await decrypt(thumbEnc);
              const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
              thumbnailData = base64ToArrayBuffer(thumbPayload.data);
              unsyncedThumbMime = thumbPayload.mime_type;
            } catch { /* placeholder */ }
          }

          const thumbMime =
            unsyncedThumbMime || (payload.media_type === "gif" ? "image/gif" : "image/jpeg");
          directPhotos.push({
            blobId: blob.id,
            thumbnailBlobId: payload.thumbnail_blob_id,
            filename: payload.filename,
            takenAt: new Date(payload.taken_at).getTime(),
            mimeType: payload.mime_type,
            mediaType: payload.media_type ?? mediaTypeFromMime(payload.mime_type),
            width: payload.width,
            height: payload.height,
            duration: payload.duration,
            latitude: payload.latitude,
            longitude: payload.longitude,
            albumIds: payload.album_ids ?? [],
            contentHash: blob.content_hash ?? undefined,
            ...(thumbnailData ? { thumbnailMimeType: thumbMime } : {}),
          });
          if (thumbnailData) {
            directThumbs.push({ blobId: blob.id, data: thumbnailData, mime: thumbMime });
          }
          if (directPhotos.length >= RECONCILE_CHUNK) await flushDirect();
        } catch {
          // Skip undecryptable items
        }
      }
      await flushDirect();
    } catch (err: unknown) {
      // Propagate to caller — useGalleryData will set the error
      throw err;
    } finally {
      setLoading(false);
    }
  }

  return { encryptedPhotos, loading, loadEncryptedPhotos };
}
