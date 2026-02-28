import { useRef, useCallback, useEffect } from "react";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { db, type MediaType } from "../db";
import { decrypt } from "../crypto/crypto";
import { base64ToUint8Array } from "../utils/media";

export interface PreloadEntry {
  url: string;
  filename: string;
  mimeType: string;
  mediaType: MediaType;
  cropData: { x: number; y: number; width: number; height: number; rotate: number; brightness?: number } | null;
  isFavorite: boolean;
}

/** Encrypted blob payload shape (needed for preload decryption) */
interface MediaPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type?: MediaType;
  width: number;
  height: number;
  duration?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64-encoded raw file bytes
}

export default function usePhotoPreload(
  photoIds: string[] | undefined,
  currentIndex: number,
  isPlainMode: boolean,
) {
  const preloadCache = useRef<Map<string, PreloadEntry>>(new Map());

  // Cached photo list to avoid re-fetching metadata on every preload (plain mode)
  const photoListCache = useRef<{ data: Awaited<ReturnType<typeof api.photos.list>>["photos"]; ts: number } | null>(null);

  /** Get the photo list, using a short-lived cache (30s) to avoid duplicate fetches */
  async function getCachedPhotoList() {
    const now = Date.now();
    if (photoListCache.current && now - photoListCache.current.ts < 30_000) {
      return photoListCache.current.data;
    }
    const res = await api.photos.list({ limit: 500 });
    photoListCache.current = { data: res.photos, ts: now };
    return res.photos;
  }

  /** Preload a plain-mode photo into the cache (background, no state updates) */
  async function preloadPlainPhoto(photoId: string) {
    try {
      // Fetch metadata to get filename/type (uses cached list)
      const photos = await getCachedPhotoList();
      const photo = photos.find((p) => p.id === photoId);
      if (!photo) return;

      const resolvedType: MediaType =
        photo.media_type === "gif" ? "gif"
        : photo.media_type === "video" ? "video"
        : "photo";

      // Fetch the full file
      const { accessToken } = useAuthStore.getState();
      const headers: Record<string, string> = { "X-Requested-With": "SimplePhotos" };
      if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
      const fileRes = await fetch(api.photos.fileUrl(photoId), { headers });
      if (!fileRes.ok) return;
      const blob = await fileRes.blob();
      const url = URL.createObjectURL(blob);

      let photoCropData = null;
      if (photo.crop_metadata) {
        try { photoCropData = JSON.parse(photo.crop_metadata); } catch { /* ignore */ }
      }

      preloadCache.current.set(photoId, {
        url,
        filename: photo.filename,
        mimeType: photo.mime_type,
        mediaType: resolvedType,
        cropData: photoCropData,
        isFavorite: !!photo.is_favorite,
      });
    } catch {
      // Preload failures are silent — the normal load path handles errors
    }
  }

  /** Preload an encrypted photo into the cache (background, no state updates) */
  async function preloadEncryptedPhoto(blobId: string) {
    try {
      const encrypted = await api.blobs.download(blobId);
      const decrypted = await decrypt(encrypted);
      const payload: MediaPayload = JSON.parse(new TextDecoder().decode(decrypted));

      const resolvedType: MediaType =
        payload.media_type ??
        (payload.mime_type === "image/gif"
          ? "gif"
          : payload.mime_type.startsWith("video/")
          ? "video"
          : "photo");

      const bytes = base64ToUint8Array(payload.data).buffer as ArrayBuffer;
      const blob = new Blob([bytes], { type: payload.mime_type });
      const url = URL.createObjectURL(blob);

      // Load crop data from IndexedDB
      let photoCropData = null;
      const dbEntry = await db.photos.get(blobId);
      if (dbEntry?.cropData) {
        try { photoCropData = JSON.parse(dbEntry.cropData); } catch { /* ignore */ }
      }

      preloadCache.current.set(blobId, {
        url,
        filename: payload.filename,
        mimeType: payload.mime_type,
        mediaType: resolvedType,
        cropData: photoCropData,
        isFavorite: false,
      });
    } catch {
      // Preload failures are silent
    }
  }

  // Preloads ±2 photos around the current index for instant swiping.
  // Each preloaded photo is stored as an ObjectURL in the cache ref.
  const preloadAdjacentPhotos = useCallback(
    (currentId: string) => {
      if (!photoIds || currentIndex < 0) return;

      // Determine which IDs to preload: 2 before, 2 after
      const idsToPreload: string[] = [];
      for (let offset = -2; offset <= 2; offset++) {
        if (offset === 0) continue; // skip current
        const idx = currentIndex + offset;
        if (idx >= 0 && idx < photoIds.length) {
          idsToPreload.push(photoIds[idx]);
        }
      }

      // Evict cache entries that are now too far away (beyond ±3)
      const keepSet = new Set<string>([currentId, ...idsToPreload]);
      for (const [cachedId, entry] of preloadCache.current.entries()) {
        if (!keepSet.has(cachedId) && cachedId !== currentId) {
          URL.revokeObjectURL(entry.url);
          preloadCache.current.delete(cachedId);
        }
      }

      // Kick off preloads for any not already cached
      for (const preloadId of idsToPreload) {
        if (preloadCache.current.has(preloadId)) continue; // already cached

        if (isPlainMode) {
          preloadPlainPhoto(preloadId);
        } else {
          preloadEncryptedPhoto(preloadId);
        }
      }
    },
    [photoIds, currentIndex, isPlainMode]
  );

  // Clean up all preload cache entries on unmount
  useEffect(() => {
    return () => {
      for (const entry of preloadCache.current.values()) {
        URL.revokeObjectURL(entry.url);
      }
      preloadCache.current.clear();
    };
  }, []);

  return {
    preloadCache,
    getCachedPhotoList,
    preloadAdjacentPhotos,
  };
}
