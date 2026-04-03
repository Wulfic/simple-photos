/**
 * Hook for loading and managing gallery data in encrypted mode.
 *
 * Handles cursor-based pagination, date-group detection, and encrypted sync
 * (fetching blob IDs then decrypting thumbnails from IDB).
 */
import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, hasCryptoKey } from "../crypto/crypto";
import {
  db,
  type CachedPhoto,
  type MediaType,
  mediaTypeFromMime,
} from "../db";
import { base64ToArrayBuffer } from "../utils/media";
import { fetchAllPages } from "../utils/gallery";
import { useLiveQuery } from "dexie-react-hooks";
import type { PhotoPayload, ThumbnailPayload } from "../types/media";
export type { PhotoPayload, ThumbnailPayload };

export interface GalleryDataResult {
  loading: boolean;
  error: string;
  setError: (msg: string) => void;
  /** Encrypted-mode photos from IndexedDB (live query, auto-updates).
   *  Returns undefined until the first server sync completes to prevent
   *  flashing stale data from a previous user's session. */
  encryptedPhotos: CachedPhoto[] | undefined;
  secureBlobIds: Set<string>;
  loadEncryptedPhotos: () => Promise<void>;
}

/**
 * Core data hook for the Gallery page.
 *
 * Always operates in encrypted mode. Loads encrypted photos from IndexedDB.
 */
export function useGalleryData(): GalleryDataResult {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [secureBlobIds, setSecureBlobIds] = useState<Set<string>>(new Set());
  const navigate = useNavigate();

  // Tracks whether the first server sync has completed for encrypted mode.
  // Until this is true, we suppress the IndexedDB live query results to
  // prevent flashing stale data from a previous user's session.
  const [encryptedDataReady, setEncryptedDataReady] = useState(false);

  // Encrypted-mode: cached photos from IndexedDB (live query)
  const rawEncryptedPhotos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

  // Gate: only expose encrypted photos after the first server sync
  const encryptedPhotos = encryptedDataReady ? rawEncryptedPhotos : undefined;

  // ── Periodic secure-blob-ID refresh ─────────────────────────────────────
  // Photos may be moved to/from secure galleries on other devices. Refresh
  // the set every few seconds so the main gallery hides/shows them promptly.
  const securePollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  function startSecureBlobIdPolling() {
    if (securePollRef.current) return;
    securePollRef.current = setInterval(async () => {
      try {
        const res = await api.secureGalleries.secureBlobIds();
        const fresh = new Set(res.blob_ids);
        setSecureBlobIds((prev) => {
          // Only update state if the set actually changed (avoids re-renders)
          if (prev.size !== fresh.size) {
            console.log(`[DIAG:SECURE_POLL] secureBlobIds changed: ${prev.size} → ${fresh.size}, ids:`, Array.from(fresh));
            return fresh;
          }
          for (const id of fresh) {
            if (!prev.has(id)) {
              console.log(`[DIAG:SECURE_POLL] secureBlobIds changed (new member), ids:`, Array.from(fresh));
              return fresh;
            }
          }
          return prev;
        });
      } catch {
        // Non-critical — ignore transient failures
      }
    }, 5_000);
  }

  // Cleanup polling on unmount
  useEffect(() => {
    return () => {
      if (securePollRef.current) {
        clearInterval(securePollRef.current);
        securePollRef.current = null;
      }
    };
  }, []);

  // ── Initialization ──────────────────────────────────────────────────────

  useEffect(() => {
    async function init() {
      try {

        // Fetch blob IDs that are in secure galleries (to hide from main gallery)
        try {
          const secureRes = await api.secureGalleries.secureBlobIds();
          setSecureBlobIds(new Set(secureRes.blob_ids));
        } catch {
          // Secure galleries may not be available — ignore
        }

        // Start periodic refresh of secureBlobIds so photos moved to/from
        // secure galleries on other devices are reflected without a full reload.
        startSecureBlobIdPolling();

        if (!hasCryptoKey()) {
          navigate("/setup");
          return;
        }
        await loadEncryptedPhotos();
      } catch (err) {
        console.error("Failed to initialize gallery:", err);
        setError("Failed to load gallery. Please try again.");
      }
    }
    init();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- Intentionally runs once on mount.
    // All dependencies (loadEncryptedPhotos, navigate) are stable callbacks/functions
    // defined in this hook. Including them would cause infinite re-fetches because
    // the loaders update state that the effect closes over.
  }, []);

  // ── Load encrypted-mode photos from blobs + IndexedDB ──────────────────

  async function loadEncryptedPhotos() {
    setLoading(true);
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

      // Build set of all valid server IDs and Blob IDs
      const serverPhotoIds = new Set<string>();
      const serverBlobIds = new Set<string>();
      for (const p of allSyncPhotos) {
        serverPhotoIds.add(p.id);
        if (p.encrypted_blob_id) {
          serverBlobIds.add(p.encrypted_blob_id);
        }
      }

      // Phase 2: Also include directly-uploaded encrypted blobs not yet
      // registered in the photos table (no encrypted_blob_id link).
      const allBlobMedia = [
        ...(await fetchAllPages("photo")),
        ...(await fetchAllPages("gif")),
        ...(await fetchAllPages("video")),
      ];
      for (const blob of allBlobMedia) {
        serverBlobIds.add(blob.id);
      }

      // Remove stale entries from IndexedDB that no longer exist on server
      const currentCached = await db.photos.toArray();
      const staleIds = currentCached
        .filter((p) => {
          // If it is bound to a server photo record, the server must still have it
          if (p.serverPhotoId) {
            const stale = !serverPhotoIds.has(p.serverPhotoId);
            if (stale) console.log("[Gallery:stale] serverPhotoId not found:", p.blobId, p.serverPhotoId);
            return stale;
          }
          // Server-side (autoscanned) photos use their photo ID as blobId
          if (p.serverSide) {
            const stale = !serverPhotoIds.has(p.blobId);
            if (stale) console.log("[Gallery:stale] serverSide blobId not found:", p.blobId);
            return stale;
          }
          // For direct uploads or unsynced local copies check both blob IDs and
          // server photo IDs — storageBlobId can reference either an encrypted
          // blob or a server-side autoscanned photo (no blob to look up).
          const underlyingId = p.storageBlobId || p.blobId;
          const stale = !serverBlobIds.has(underlyingId) && !serverPhotoIds.has(underlyingId);
          if (stale) console.log("[Gallery:stale] blob/photo not found:", p.blobId, "underlyingId:", underlyingId);
          return stale;
        })
        .map((p) => p.blobId);

      if (staleIds.length > 0) {
        console.log("[Gallery:sync] Deleting stale IDB entries:", staleIds);
        await db.photos.bulkDelete(staleIds);
      }
      console.log("[Gallery:sync] Stale cleanup done. Remaining:", currentCached.length - staleIds.length, "of", currentCached.length);

      // Stale data is purged — safe to expose the live query to the UI.
      setEncryptedDataReady(true);

      // We re-query since we might have deleted some
      const survivingCached = await db.photos.toArray();
      const idbByServerId = new Map(
        survivingCached.filter((p) => p.serverPhotoId).map((p) => [p.serverPhotoId!, p])
      );
      const idbByBlobId = new Map(survivingCached.map((p) => [p.blobId, p]));

      // Phase 3: Populate IndexedDB from sync records.
      // Skip autoscanned photos (no encrypted_blob_id) — encrypted-only mode.
      for (const photo of allSyncPhotos) {
        if (!photo.encrypted_blob_id) continue; // Skip plain-mode autoscanned entries

        let existing = idbByServerId.get(photo.id);

        if (!existing && photo.encrypted_blob_id) {
          // Try to bind an unbound unsynced original upload
          const boundByBlob = idbByBlobId.get(photo.encrypted_blob_id);
          if (boundByBlob && !boundByBlob.serverPhotoId) {
            existing = boundByBlob;
            existing.serverPhotoId = photo.id;
            idbByServerId.set(photo.id, existing); // Prevent another duplicate from claiming it
          }
        }

        // If creating a NEW DB record for a duplicate that wasn't found locally,
        // use photo.id as its unique key instead of clumping over encrypted_blob_id.
        const idbKey = existing ? existing.blobId : photo.id;

        if (existing) {
          // Update mutable server-synced fields (favorites, serverPhotoId)
          const updates: Partial<CachedPhoto> = {};
          if (existing.isFavorite !== photo.is_favorite) updates.isFavorite = photo.is_favorite;
          if (existing.serverPhotoId !== photo.id) updates.serverPhotoId = photo.id;
          // Retry thumbnail download if the previous attempt failed
          if (!existing.thumbnailData) {
            const retryThumbId = existing.thumbnailBlobId ?? photo.encrypted_thumb_blob_id;
            if (retryThumbId) {
              try {
                const thumbEnc = await api.blobs.download(retryThumbId);
                const thumbDec = await decrypt(thumbEnc);
                const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
                const retryData = base64ToArrayBuffer(thumbPayload.data);
                updates.thumbnailData = retryData;
                updates.thumbnailMimeType = thumbPayload.mime_type;
              } catch {
                // Still unavailable — leave as-is and retry next sync
              }
            }
          }
          if (Object.keys(updates).length > 0) await db.photos.update(idbKey, updates);
          continue;
        }

        // Parse takenAt: prefer taken_at, fall back to created_at (matches Android)
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

        // Encrypted photo — download and decrypt thumbnail blob
        const thumbBlobId = photo.encrypted_thumb_blob_id;
        if (thumbBlobId) {
          try {
            const thumbEnc = await api.blobs.download(thumbBlobId);
            const thumbDec = await decrypt(thumbEnc);
            const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
            thumbnailData = base64ToArrayBuffer(thumbPayload.data);
            thumbnailMimeType = thumbPayload.mime_type;
          } catch {
            // Thumbnail fetch failed — show placeholder
          }
        }

        await db.photos.put({
          blobId: idbKey,
          storageBlobId: photo.encrypted_blob_id ?? undefined,
          thumbnailBlobId: photo.encrypted_thumb_blob_id ?? undefined,
          filename: photo.filename,
          takenAt,
          mimeType: photo.mime_type,
          mediaType: (photo.media_type as MediaType) ?? mediaTypeFromMime(photo.mime_type),
          width: photo.width,
          height: photo.height,
          duration: photo.duration_secs ?? undefined,
          albumIds: [],
          thumbnailData,
          thumbnailMimeType,
          contentHash: photo.photo_hash ?? undefined,
          cropData: photo.crop_metadata ?? undefined,
          isFavorite: photo.is_favorite ?? false,
          serverPhotoId: photo.id,
        });
      }

      // Phase 4: Handle directly-uploaded encrypted blobs not in photos table.
      // These require full blob decryption to extract metadata.
      const syncedBlobIds = new Set(
        allSyncPhotos
          .map((p) => p.encrypted_blob_id)
          .filter((id): id is string => !!id)
      );
      const unsyncedBlobs = allBlobMedia.filter((b) => !syncedBlobIds.has(b.id));

      for (const blob of unsyncedBlobs) {
        const existing = await db.photos.get(blob.id);
        if (existing) continue;

        try {
          const encrypted = await api.blobs.download(blob.id);
          const decrypted = await decrypt(encrypted);
          const payload: PhotoPayload = JSON.parse(new TextDecoder().decode(decrypted));

          let thumbnailData: ArrayBuffer | undefined;
          let unsyncedThumbMime: string | undefined;
          if (payload.thumbnail_blob_id) {
            try {
              const thumbEnc = await api.blobs.download(payload.thumbnail_blob_id);
              const thumbDec = await decrypt(thumbEnc);
              const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
              thumbnailData = base64ToArrayBuffer(thumbPayload.data);
              unsyncedThumbMime = thumbPayload.mime_type;
            } catch {
              // Thumbnail fetch failed — show placeholder
            }
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
            thumbnailData,
            thumbnailMimeType: unsyncedThumbMime,
            contentHash: blob.content_hash ?? undefined,
          });
        } catch {
          // Skip items we can't decrypt (wrong key or corrupt blob)
        }
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load");
    } finally {
      setLoading(false);
    }
  }

  return {
    loading,
    error,
    setError,
    encryptedPhotos,
    secureBlobIds,
    loadEncryptedPhotos,
  };
}
