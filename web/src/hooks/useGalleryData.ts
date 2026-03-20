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
  data: string; // base64 JPEG
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
          if (prev.size !== fresh.size) return fresh;
          for (const id of fresh) {
            if (!prev.has(id)) return fresh;
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
      // Phase 1: Fetch metadata via encrypted-sync endpoint
      // Uses the same server-side data source as the Android app, ensuring
      // consistent sort order (COALESCE(taken_at, created_at) DESC).
      type SyncRecord = Awaited<ReturnType<typeof api.photos.encryptedSync>>["photos"][number];
      const allSyncPhotos: SyncRecord[] = [];
      let cursor: string | undefined;
      do {
        const res = await api.photos.encryptedSync({ after: cursor, limit: 500 });
        allSyncPhotos.push(...res.photos);
        cursor = res.next_cursor ?? undefined;
      } while (cursor);

      // Build set of server blob IDs for stale-entry cleanup
      const serverBlobIds = new Set(
        allSyncPhotos.map((p) => p.encrypted_blob_id).filter(Boolean) as string[]
      );

      // Phase 2: Also fetch blobs list for directly-uploaded encrypted
      // photos (not yet in photos table — no encrypted_blob_id link).
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
      // Any remaining cached entries belong to the current user.
      setEncryptedDataReady(true);

      // Phase 3: Populate IndexedDB from sync records (migrated photos).
      // Only download small thumbnail blobs (~30 KB) — not full photos.
      for (const photo of allSyncPhotos) {
        const blobId = photo.encrypted_blob_id;
        if (!blobId) continue;

        const existing = await db.photos.get(blobId);
        if (existing) {
          // Update mutable server-synced fields on existing records
          // (favorite may have changed on another device, serverPhotoId may be missing)
          const updates: Partial<CachedPhoto> = {};
          if (existing.isFavorite !== photo.is_favorite) updates.isFavorite = photo.is_favorite;
          if (existing.serverPhotoId !== photo.id) updates.serverPhotoId = photo.id;
          if (Object.keys(updates).length > 0) await db.photos.update(blobId, updates);
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

        // Download and decrypt thumbnail blob if available
        // TODO: Consider using the server's GET /api/blobs/{id}/thumb endpoint
        // which resolves photo-blob-ID → thumb automatically, instead of
        // requiring the client to know the thumb blob ID up front.
        let thumbnailData: ArrayBuffer | undefined;
        const thumbBlobId = photo.encrypted_thumb_blob_id;
        if (thumbBlobId) {
          try {
            const thumbEnc = await api.blobs.download(thumbBlobId);
            const thumbDec = await decrypt(thumbEnc);
            const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
            thumbnailData = base64ToArrayBuffer(thumbPayload.data);
          } catch {
            // Thumbnail fetch failed — show placeholder
          }
        }

        await db.photos.put({
          blobId,
          thumbnailBlobId: thumbBlobId ?? undefined,
          filename: photo.filename,
          takenAt,
          mimeType: photo.mime_type,
          mediaType: (photo.media_type as MediaType) ?? mediaTypeFromMime(photo.mime_type),
          width: photo.width,
          height: photo.height,
          duration: photo.duration_secs ?? undefined,
          albumIds: [],
          thumbnailData,
          contentHash: photo.photo_hash ?? undefined,
          cropData: photo.crop_metadata ?? undefined,
          isFavorite: photo.is_favorite ?? false,
          serverPhotoId: photo.id,
        });
      }

      // Phase 4: Handle directly-uploaded encrypted blobs not in photos table.
      // These require full blob decryption to extract metadata.
      const syncedBlobIds = new Set(
        allSyncPhotos.map((p) => p.encrypted_blob_id).filter(Boolean)
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
          if (payload.thumbnail_blob_id) {
            try {
              const thumbEnc = await api.blobs.download(payload.thumbnail_blob_id);
              const thumbDec = await decrypt(thumbEnc);
              const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
              thumbnailData = base64ToArrayBuffer(thumbPayload.data);
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
