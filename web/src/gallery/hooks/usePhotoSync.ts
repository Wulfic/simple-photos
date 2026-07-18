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
  type MediaType,
  mediaTypeFromMime,
} from "../../db";
import { putThumb, resolveThumb, startThumbBackfill } from "../../db/thumbs";
import { base64ToArrayBuffer } from "../../utils/media";
import { fetchAllPages } from "../../utils/gallery";
import { decodeThumbnailDimensions } from "../utils/thumbnailGenerate";
import {
  isTransposed as checkTransposed,
  correctDimensionsFromThumbnail,
  queueDimensionUpdate,
} from "./useDimensionSync";
import { useLiveQuery } from "dexie-react-hooks";
import type { PhotoPayload, ThumbnailPayload } from "../../types/media";

/**
 * Ensure an already-cached photo has its thumbnail bytes stored locally,
 * downloading them **only when they're actually missing**.
 *
 * This is the anti-thrash guard behind the periodic sync: a photo whose
 * thumbnail already resolves from the `thumbs` table is left untouched, so a
 * steady-state sync pass over a fully-cached library performs zero blob
 * downloads. Before the thumbs split + sync-interval fix, overlapping 2-second
 * syncs raced each other's writes and re-downloaded thumbnails the racing pass
 * hadn't yet persisted — ~28 blob downloads/second against the server
 * (repo todo.md, "Bug B — web client runs a FULL library sync every 2s").
 *
 * Returns `true` iff a download + store actually happened (for tests / callers
 * that count). Kept module-level and exported so the decision can be unit-tested
 * directly, rather than proven only through a full hook render.
 */
export async function ensureThumbCached(
  existing: CachedPhoto,
  serverThumbId: string | undefined,
): Promise<boolean> {
  // Already have the bytes locally — the guard. Never re-fetch a thumbnail a
  // previous pass already persisted.
  if (await resolveThumb(existing)) return false;
  const thumbId = existing.thumbnailBlobId ?? serverThumbId;
  if (!thumbId) return false;
  try {
    const thumbEnc = await api.blobs.download(thumbId);
    const thumbDec = await decrypt(thumbEnc);
    const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
    await putThumb(
      existing.blobId,
      base64ToArrayBuffer(thumbPayload.data),
      thumbPayload.mime_type,
      existing.mediaType,
    );
    return true;
  } catch {
    return false; // leave the cache as-is; a later pass retries
  }
}

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
      type SyncRecord = Awaited<ReturnType<typeof api.photos.encryptedSync>>["photos"][number];
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
      const staleIds = currentCached
        .filter((p) => {
          if (p.serverPhotoId) return !serverPhotoIds.has(p.serverPhotoId);
          const underlyingId = p.storageBlobId || p.blobId;
          return !serverBlobIds.has(underlyingId) && !serverPhotoIds.has(underlyingId);
        })
        .map((p) => p.blobId);
      if (staleIds.length > 0) await db.photos.bulkDelete(staleIds);

      setEncryptedDataReady(true);

      const survivingCached = await db.photos.toArray();
      const idbByServerId = new Map(
        survivingCached.filter((p) => p.serverPhotoId).map((p) => [p.serverPhotoId!, p]),
      );
      const idbByBlobId = new Map(survivingCached.map((p) => [p.blobId, p]));

      // Phase 3: Populate IDB from sync records.
      for (const photo of allSyncPhotos) {
        if (!photo.encrypted_blob_id) continue;

        let existing = idbByServerId.get(photo.id);
        if (!existing && photo.encrypted_blob_id) {
          const boundByBlob = idbByBlobId.get(photo.encrypted_blob_id);
          if (boundByBlob && !boundByBlob.serverPhotoId) {
            existing = boundByBlob;
            existing.serverPhotoId = photo.id;
            idbByServerId.set(photo.id, existing);
          }
        }

        const idbKey = existing ? existing.blobId : photo.id;

        if (existing) {
          const updates: Partial<CachedPhoto> = {};
          if (existing.isFavorite !== photo.is_favorite) updates.isFavorite = photo.is_favorite;
          if (existing.serverPhotoId !== photo.id) updates.serverPhotoId = photo.id;
          const serverBlobIdVal = photo.encrypted_blob_id ?? undefined;
          if (serverBlobIdVal && existing.storageBlobId !== serverBlobIdVal) updates.storageBlobId = serverBlobIdVal;
          const serverSourcePath = photo.source_path ?? undefined;
          if (existing.sourcePath !== serverSourcePath) updates.sourcePath = serverSourcePath;
          const serverSubtype = photo.photo_subtype ?? undefined;
          if (existing.photoSubtype !== serverSubtype) updates.photoSubtype = serverSubtype;
          // Backfill addedAt (library import order) for entries cached before
          // the field existed. created_at never changes, so set-once is safe.
          if (existing.addedAt === undefined) {
            const added = new Date(photo.created_at).getTime();
            if (!Number.isNaN(added)) updates.addedAt = added;
          }
          const serverBurstId = photo.burst_id ?? undefined;
          if (existing.burstId !== serverBurstId) updates.burstId = serverBurstId;
          const serverMotionBlob = photo.motion_video_blob_id ?? undefined;
          if (existing.motionVideoBlobId !== serverMotionBlob) updates.motionVideoBlobId = serverMotionBlob;
          const serverCrop = photo.crop_metadata ?? undefined;
          if (existing.cropData !== serverCrop) updates.cropData = serverCrop;

          // Dimension sync with transpose guard
          if (photo.width > 0 && photo.height > 0 &&
              (existing.width !== photo.width || existing.height !== photo.height)) {
            if (!checkTransposed(existing.width, existing.height, photo.width, photo.height)) {
              updates.width = photo.width;
              updates.height = photo.height;
            }
          }

          // Re-download thumbnail when server's thumb blob ID changed
          const serverThumbId = photo.encrypted_thumb_blob_id ?? undefined;
          if (serverThumbId && existing.thumbnailBlobId !== serverThumbId) {
            updates.thumbnailBlobId = serverThumbId;
            try {
              const thumbEnc = await api.blobs.download(serverThumbId);
              const thumbDec = await decrypt(thumbEnc);
              const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
              const freshData = base64ToArrayBuffer(thumbPayload.data);
              await putThumb(idbKey, freshData, thumbPayload.mime_type, existing.mediaType);
              const curW = updates.width ?? existing.width;
              const curH = updates.height ?? existing.height;
              if (curW > 0 && curH > 0) {
                try {
                  const td = await decodeThumbnailDimensions(freshData, thumbPayload.mime_type);
                  const correction = correctDimensionsFromThumbnail(td.width, td.height, curW, curH);
                  if (correction) {
                    updates.width = correction.width;
                    updates.height = correction.height;
                  }
                } catch { /* ignore */ }
              }
            } catch {
              // Download failed — leave existing thumbnail
            }
          } else {
            // Server thumb unchanged — make sure the bytes are actually cached,
            // downloading only if they're missing (the anti-thrash guard).
            await ensureThumbCached(existing, photo.encrypted_thumb_blob_id ?? undefined);
          }
          if (Object.keys(updates).length > 0) await db.photos.update(idbKey, updates);
          continue;
        }

        // New entry — parse and insert
        let takenAt: number;
        try {
          takenAt = photo.taken_at
            ? new Date(photo.taken_at).getTime()
            : new Date(photo.created_at).getTime();
        } catch {
          takenAt = new Date(photo.created_at).getTime();
        }

        let thumbnailData: ArrayBuffer | undefined;
        let thumbnailMimeType: string | undefined;
        const thumbBlobId = photo.encrypted_thumb_blob_id;
        if (thumbBlobId) {
          try {
            const thumbEnc = await api.blobs.download(thumbBlobId);
            const thumbDec = await decrypt(thumbEnc);
            const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
            thumbnailData = base64ToArrayBuffer(thumbPayload.data);
            thumbnailMimeType = thumbPayload.mime_type;
          } catch { /* placeholder */ }
        }

        let displayWidth = photo.width;
        let displayHeight = photo.height;
        if (thumbnailData && displayWidth > 0 && displayHeight > 0) {
          try {
            const thumbDims = await decodeThumbnailDimensions(thumbnailData, thumbnailMimeType);
            const correction = correctDimensionsFromThumbnail(
              thumbDims.width, thumbDims.height, displayWidth, displayHeight,
            );
            if (correction) {
              displayWidth = correction.width;
              displayHeight = correction.height;
              queueDimensionUpdate(photo.id, displayWidth, displayHeight);
            }
          } catch { /* use server dimensions */ }
        }

        await db.photos.put({
          blobId: idbKey,
          storageBlobId: photo.encrypted_blob_id ?? undefined,
          thumbnailBlobId: photo.encrypted_thumb_blob_id ?? undefined,
          filename: photo.filename,
          takenAt,
          mimeType: photo.mime_type,
          mediaType: (photo.media_type as MediaType) ?? mediaTypeFromMime(photo.mime_type),
          width: displayWidth,
          height: displayHeight,
          duration: photo.duration_secs ?? undefined,
          albumIds: [],
          contentHash: photo.photo_hash ?? undefined,
          cropData: photo.crop_metadata ?? undefined,
          isFavorite: photo.is_favorite ?? false,
          serverPhotoId: photo.id,
          sourcePath: photo.source_path ?? undefined,
          addedAt: (() => {
            const added = new Date(photo.created_at).getTime();
            return Number.isNaN(added) ? takenAt : added;
          })(),
          photoSubtype: photo.photo_subtype ?? undefined,
          burstId: photo.burst_id ?? undefined,
          motionVideoBlobId: photo.motion_video_blob_id ?? undefined,
        });
        // Thumbnail bytes go to their own table — keeping them on the photo row
        // is what made every count query deserialize the whole library.
        if (thumbnailData) {
          await putThumb(idbKey, thumbnailData, thumbnailMimeType, photo.media_type ?? undefined);
        }
      }

      // Phase 4: Handle directly-uploaded encrypted blobs not in photos table.
      const syncedBlobIds = new Set(
        allSyncPhotos.map((p) => p.encrypted_blob_id).filter((id): id is string => !!id),
      );
      const unsyncedBlobs = allBlobMedia.filter((b) => !syncedBlobIds.has(b.id));

      for (const blob of unsyncedBlobs) {
        const existing = await db.photos.get(blob.id);
        if (existing) continue;
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

          await db.photos.put({
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
          });
          if (thumbnailData) {
            await putThumb(blob.id, thumbnailData, unsyncedThumbMime, payload.media_type);
          }
        } catch {
          // Skip undecryptable items
        }
      }
    } catch (err: unknown) {
      // Propagate to caller — useGalleryData will set the error
      throw err;
    } finally {
      setLoading(false);
    }
  }

  return { encryptedPhotos, loading, loadEncryptedPhotos };
}
