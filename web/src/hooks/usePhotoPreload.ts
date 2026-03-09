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

/**
 * Preloads adjacent photos into an in-memory cache for instant swiping.
 *
 * Features:
 * - ±5 preload window (up to 10 photos preloaded around the current)
 * - Direction-aware: biases preload toward swipe direction (+5 forward, -2 back)
 * - Caches decrypted full photos in IndexedDB for cross-session persistence
 * - Evicts ObjectURLs beyond ±7 to manage memory
 */
export default function usePhotoPreload(
  photoIds: string[] | undefined,
  currentIndex: number,
  isPlainMode: boolean,
) {
  const preloadCache = useRef<Map<string, PreloadEntry>>(new Map());
  const lastDirection = useRef<"forward" | "backward">("forward");
  const prevIndex = useRef(currentIndex);

  // Track swipe direction
  useEffect(() => {
    if (currentIndex > prevIndex.current) {
      lastDirection.current = "forward";
    } else if (currentIndex < prevIndex.current) {
      lastDirection.current = "backward";
    }
    prevIndex.current = currentIndex;
  }, [currentIndex]);

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
      // Check IndexedDB full-photo cache first
      const idbCached = await db.fullPhotos?.get(photoId);
      if (idbCached?.data) {
        const blob = new Blob([idbCached.data], { type: idbCached.mimeType });
        const url = URL.createObjectURL(blob);
        preloadCache.current.set(photoId, {
          url,
          filename: idbCached.filename,
          mimeType: idbCached.mimeType,
          mediaType: idbCached.mediaType,
          cropData: idbCached.cropData ? JSON.parse(idbCached.cropData) : null,
          isFavorite: idbCached.isFavorite,
        });
        return;
      }

      // Fetch metadata to get filename/type (uses cached list)
      const photos = await getCachedPhotoList();
      const photo = photos.find((p) => p.id === photoId);
      if (!photo) return;

      const resolvedType: MediaType =
        photo.media_type === "gif" ? "gif"
        : photo.media_type === "video" ? "video"
        : photo.media_type === "audio" ? "audio"
        : "photo";

      // Fetch the full file (use /web endpoint for browser-compatible format)
      const { accessToken } = useAuthStore.getState();
      const headers: Record<string, string> = { "X-Requested-With": "SimplePhotos" };
      if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
      const fileRes = await fetch(api.photos.webUrl(photoId), { headers });
      // 202 = conversion in progress — don't cache the JSON placeholder
      if (!fileRes.ok || fileRes.status === 202) return;
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

      // Cache in IndexedDB for cross-session persistence (skip large videos > 50MB)
      if (blob.size < 50 * 1024 * 1024) {
        try {
          const arrayBuf = await blob.arrayBuffer();
          await db.fullPhotos?.put({
            photoId,
            filename: photo.filename,
            mimeType: photo.mime_type,
            mediaType: resolvedType,
            cropData: photo.crop_metadata ?? undefined,
            isFavorite: !!photo.is_favorite,
            data: arrayBuf,
            cachedAt: Date.now(),
          });
        } catch { /* IndexedDB write failure is non-fatal */ }
      }
    } catch {
      // Preload failures are silent — the normal load path handles errors
    }
  }

  /** Preload an encrypted photo into the cache (background, no state updates) */
  async function preloadEncryptedPhoto(blobId: string) {
    try {
      // Resolve the actual server blob ID (copies reference the original via storageBlobId)
      const dbEntry = await db.photos.get(blobId);
      const fetchId = dbEntry?.storageBlobId || blobId;

      // Check IndexedDB full-photo cache first (keyed by fetchId for dedup)
      const idbCached = await db.fullPhotos?.get(fetchId);
      if (idbCached?.data) {
        const blob = new Blob([idbCached.data], { type: idbCached.mimeType });
        const url = URL.createObjectURL(blob);
        preloadCache.current.set(blobId, {
          url,
          filename: dbEntry?.filename ?? idbCached.filename,
          mimeType: idbCached.mimeType,
          mediaType: idbCached.mediaType,
          cropData: dbEntry?.cropData ? JSON.parse(dbEntry.cropData) : null,
          isFavorite: idbCached.isFavorite,
        });
        return;
      }

      const encrypted = await api.blobs.download(fetchId);
      const decrypted = await decrypt(encrypted);
      const payload: MediaPayload = JSON.parse(new TextDecoder().decode(decrypted));

      const resolvedType: MediaType =
        payload.media_type ??
        (payload.mime_type === "image/gif"
          ? "gif"
          : payload.mime_type.startsWith("video/")
          ? "video"
          : payload.mime_type.startsWith("audio/")
          ? "audio"
          : "photo");

      const bytes = base64ToUint8Array(payload.data).buffer as ArrayBuffer;
      const blob = new Blob([bytes], { type: payload.mime_type });
      const url = URL.createObjectURL(blob);

      // Load crop data from the already-fetched dbEntry
      let photoCropData = null;
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

      // Cache in IndexedDB for cross-session persistence (skip large videos > 50MB)
      // Use fetchId as key so copies share the same cached blob data
      if (bytes.byteLength < 50 * 1024 * 1024) {
        try {
          await db.fullPhotos?.put({
            photoId: fetchId,
            filename: payload.filename,
            mimeType: payload.mime_type,
            mediaType: resolvedType,
            cropData: dbEntry?.cropData,
            isFavorite: false,
            data: bytes,
            cachedAt: Date.now(),
          });
        } catch { /* IndexedDB write failure is non-fatal */ }
      }
    } catch {
      // Preload failures are silent
    }
  }

  /**
   * Preloads photos around the current index for instant swiping.
   *
   * Direction-aware: when swiping forward, preload +5 ahead / -2 behind.
   * When swiping backward, preload -5 behind / +2 ahead.
   * This prioritizes what the user is most likely to view next.
   */
  const preloadAdjacentPhotos = useCallback(
    (currentId: string) => {
      if (!photoIds || currentIndex < 0) return;

      // Direction-aware window: bias toward swipe direction
      const isForward = lastDirection.current === "forward";
      const aheadCount = isForward ? 5 : 2;
      const behindCount = isForward ? 2 : 5;

      const idsToPreload: string[] = [];
      for (let offset = -behindCount; offset <= aheadCount; offset++) {
        if (offset === 0) continue; // skip current
        const idx = currentIndex + offset;
        if (idx >= 0 && idx < photoIds.length) {
          idsToPreload.push(photoIds[idx]);
        }
      }

      // Evict cache entries that are now too far away (beyond ±7)
      const evictThreshold = 7;
      const keepSet = new Set<string>([currentId, ...idsToPreload]);
      for (const [cachedId, entry] of preloadCache.current.entries()) {
        if (!keepSet.has(cachedId)) {
          const cachedIdx = photoIds.indexOf(cachedId);
          if (cachedIdx === -1 || Math.abs(cachedIdx - currentIndex) > evictThreshold) {
            URL.revokeObjectURL(entry.url);
            preloadCache.current.delete(cachedId);
          }
        }
      }

      // Kick off preloads — prioritize by distance from current index
      const sortedIds = idsToPreload
        .filter((id) => !preloadCache.current.has(id))
        .sort((a, b) => {
          const idxA = photoIds.indexOf(a);
          const idxB = photoIds.indexOf(b);
          return Math.abs(idxA - currentIndex) - Math.abs(idxB - currentIndex);
        });

      for (const preloadId of sortedIds) {
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
