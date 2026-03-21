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

// ── Encrypted payload shapes (shared with upload hook) ───────────────────────

export interface PhotoPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type: "photo" | "gif" | "video" | "audio";
  width: number;
  height: number;
  duration?: number;
  latitude?: number;
  longitude?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64
}

export interface ThumbnailPayload {
  v: number;
  photo_blob_id: string;
  width: number;
  height: number;
  /** Optional MIME type of the thumbnail data (defaults to "image/jpeg" for backwards compat) */
  mime_type?: string;
  data: string; // base64 JPEG or GIF
}

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

        // Fire auto-scan in the background — don't block photo loading.
        // When it completes, reload photos so newly scanned files appear.
        const reloadAfterScan = async () => {
          try {
            await loadEncryptedPhotos();
          } catch {
            // Non-critical — ignore transient failures
          }
        };
        api.backup.triggerAutoScan()
          .then(() => reloadAfterScan())
          .catch(() => {
            // Non-critical — if the user isn't admin or endpoint fails, just ignore
          });

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

      // Build set of all valid IDB keys from the server.
      // Includes both encrypted blob IDs and autoscanned photo IDs.
      const serverBlobIds = new Set<string>();
      for (const p of allSyncPhotos) {
        if (p.encrypted_blob_id) {
          serverBlobIds.add(p.encrypted_blob_id);
        } else {
          // Autoscanned (server-side) photo — keyed by photo.id
          serverBlobIds.add(p.id);
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
      const cachedPhotos = await db.photos.toArray();
      const staleIds = cachedPhotos
        .map((p) => p.blobId)
        .filter((id) => !serverBlobIds.has(id));
      if (staleIds.length > 0) {
        await db.photos.bulkDelete(staleIds);
      }

      // Stale data is purged — safe to expose the live query to the UI.
      setEncryptedDataReady(true);

      // Phase 3: Populate IndexedDB from sync records.
      for (const photo of allSyncPhotos) {
        const blobId = photo.encrypted_blob_id;
        const isServerSide = !blobId;
        // For encrypted photos, use the blob ID as IDB key.
        // For autoscanned (server-side) photos, use the photo.id.
        const idbKey = blobId || photo.id;

        const existing = await db.photos.get(idbKey);
        if (existing) {
          // Update mutable server-synced fields (favorites, serverPhotoId)
          const updates: Partial<CachedPhoto> = {};
          if (existing.isFavorite !== photo.is_favorite) updates.isFavorite = photo.is_favorite;
          if (existing.serverPhotoId !== photo.id) updates.serverPhotoId = photo.id;
          // Retry thumbnail download if the previous attempt failed (thumbnailData
          // is absent but the thumbnail blob ID is known).  This repairs photos
          // whose thumbnail was not fetched due to a transient network error.
          if (!existing.thumbnailData && !isServerSide) {
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

        if (isServerSide) {
          // Autoscanned photo — do not pre-download here to avoid blocking sync.
          // MediaTile will fetch it directly using the thumbnail endpoint.
          thumbnailMimeType = photo.mime_type === "image/gif" ? "image/gif" : "image/jpeg";
        } else {
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
        }

        await db.photos.put({
          blobId: idbKey,
          thumbnailBlobId: isServerSide ? undefined : (photo.encrypted_thumb_blob_id ?? undefined),
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
          serverSide: isServerSide || undefined,
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
