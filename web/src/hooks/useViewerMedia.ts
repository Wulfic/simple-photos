/**
 * Hook that encapsulates media loading logic for the Viewer page.
 *
 * Handles both plain-mode (fetch from server) and encrypted-mode
 * (download + decrypt) media loading, with IndexedDB caching for
 * instant display on return visits and cross-session persistence.
 */
import { useState, useCallback, useRef } from "react";
import { api } from "../api/client";
import { decrypt } from "../crypto/crypto";
import { useAuthStore } from "../store/auth";
import { db, type MediaType } from "../db";
import { base64ToUint8Array } from "../utils/media";
import type { PlainPhoto } from "../utils/gallery";

// ── Payload shape (encrypted mode) ───────────────────────────────────────────
export interface MediaPayload {
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

export interface PreloadEntry {
  url: string;
  filename: string;
  mimeType: string;
  mediaType: MediaType;
  cropData: CropMetadata | null;
  isFavorite: boolean;
}

export interface CropMetadata {
  x: number;
  y: number;
  width: number;
  height: number;
  rotate: number;
  brightness?: number;
  trimStart?: number;
  trimEnd?: number;
}

export interface PhotoInfoData {
  filename: string;
  mimeType: string;
  width?: number;
  height?: number;
  takenAt?: string | null;
  sizeBytes?: number;
  latitude?: number | null;
  longitude?: number | null;
  createdAt?: string;
  durationSecs?: number | null;
  cameraModel?: string | null;
  albumNames?: string[];
}

interface UseViewerMediaResult {
  mediaUrl: string | null;
  setMediaUrl: React.Dispatch<React.SetStateAction<string | null>>;
  previewUrl: string | null;
  setPreviewUrl: React.Dispatch<React.SetStateAction<string | null>>;
  filename: string;
  setFilename: React.Dispatch<React.SetStateAction<string>>;
  mimeType: string;
  setMimeType: React.Dispatch<React.SetStateAction<string>>;
  mediaType: MediaType;
  setMediaType: React.Dispatch<React.SetStateAction<MediaType>>;
  loading: boolean;
  setLoading: React.Dispatch<React.SetStateAction<boolean>>;
  error: string;
  setError: React.Dispatch<React.SetStateAction<string>>;
  videoError: boolean;
  setVideoError: React.Dispatch<React.SetStateAction<boolean>>;
  isConverting: boolean;
  setIsConverting: React.Dispatch<React.SetStateAction<boolean>>;
  loadPlainMedia: (photoId: string) => Promise<void>;
  loadEncryptedMedia: (blobId: string) => Promise<void>;
  preloadCacheRef: React.MutableRefObject<Map<string, PreloadEntry>>;
}

export default function useViewerMedia(
  getCachedPhotoList: () => Promise<PlainPhoto[]>,
  preloadCache: React.MutableRefObject<Map<string, PreloadEntry>>,
): UseViewerMediaResult {
  const [mediaUrl, setMediaUrl] = useState<string | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [filename, setFilename] = useState("");
  const [mimeType, setMimeType] = useState("image/jpeg");
  const [mediaType, setMediaType] = useState<MediaType>("photo");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [videoError, setVideoError] = useState(false);
  const [isConverting, setIsConverting] = useState(false);

  /** Load a plain-mode photo — check IndexedDB cache first, then fetch */
  const loadPlainMedia = useCallback(async (photoId: string) => {
    setLoading(true);
    setError("");
    try {
      // Fetch photo metadata to get filename and media type (uses cached list)
      const photos = await getCachedPhotoList();
      const photo = photos.find((p) => p.id === photoId);
      let resolvedFilename = "";
      let resolvedMime = "image/jpeg";
      let resolved: MediaType = "photo";
      let photoCropData = null;
      let photoIsFavorite = false;
      if (photo) {
        resolvedFilename = photo.filename;
        resolvedMime = photo.mime_type;
        resolved =
          photo.media_type === "gif" ? "gif"
          : photo.media_type === "video" ? "video"
          : photo.media_type === "audio" ? "audio"
          : "photo";
        photoIsFavorite = !!photo.is_favorite;
        if (photo.crop_metadata) {
          try { photoCropData = JSON.parse(photo.crop_metadata); } catch { /* ignore */ }
        }
        setFilename(resolvedFilename);
        setMimeType(resolvedMime);
        setMediaType(resolved);
      }

      // Check IndexedDB full-photo cache for instant display
      const idbCached = await db.fullPhotos?.get(photoId);
      if (idbCached?.data) {
        const blob = new Blob([idbCached.data], { type: idbCached.mimeType });
        const url = URL.createObjectURL(blob);
        setMediaUrl(url);
        preloadCache.current.set(photoId, {
          url, filename: resolvedFilename, mimeType: resolvedMime,
          mediaType: resolved, cropData: photoCropData, isFavorite: photoIsFavorite,
        });
        setLoading(false);
        return;
      }

      // Cache miss — fetch from server (use /web endpoint for browser-compatible format)
      const { accessToken } = useAuthStore.getState();
      const headers: Record<string, string> = { "X-Requested-With": "SimplePhotos" };
      if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
      const fileRes = await fetch(api.photos.webUrl(photoId), { headers });

      // 202 = conversion in progress (non-browser-native format being processed)
      if (fileRes.status === 202) {
        setIsConverting(true);
        setLoading(false);
        return;
      }

      if (!fileRes.ok) throw new Error(`Failed to load photo: ${fileRes.status}`);
      const blob = await fileRes.blob();
      const url = URL.createObjectURL(blob);
      setMediaUrl(url);

      // Store in preload cache so swiping back is instant
      preloadCache.current.set(photoId, {
        url,
        filename: resolvedFilename,
        mimeType: resolvedMime,
        mediaType: resolved,
        cropData: photoCropData,
        isFavorite: photoIsFavorite,
      });

      // Also cache in IndexedDB for cross-session persistence
      if (blob.size < 50 * 1024 * 1024) {
        try {
          const arrayBuf = await blob.arrayBuffer();
          await db.fullPhotos?.put({
            photoId, filename: resolvedFilename, mimeType: resolvedMime,
            mediaType: resolved, cropData: photo?.crop_metadata ?? undefined,
            isFavorite: photoIsFavorite, data: arrayBuf, cachedAt: Date.now(),
          });
        } catch { /* non-fatal */ }
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load media");
    } finally {
      setLoading(false);
    }
  }, [getCachedPhotoList, preloadCache]);

  /** Load an encrypted blob — check IndexedDB cache first, then decrypt */
  const loadEncryptedMedia = useCallback(async (blobId: string) => {
    setLoading(true);
    setError("");
    try {
      // Check IndexedDB full-photo cache for instant display
      const idbCached = await db.fullPhotos?.get(blobId);
      if (idbCached?.data) {
        const blob = new Blob([idbCached.data], { type: idbCached.mimeType });
        const url = URL.createObjectURL(blob);
        setMediaUrl(url);
        setFilename(idbCached.filename);
        setMimeType(idbCached.mimeType);
        const resolvedType: MediaType =
          idbCached.mediaType === "gif" ? "gif"
          : idbCached.mediaType === "video" ? "video"
          : idbCached.mediaType === "audio" ? "audio"
          : "photo";
        setMediaType(resolvedType);

        let photoCropData = null;
        if (idbCached.cropData) {
          try { photoCropData = JSON.parse(idbCached.cropData); } catch { /* ignore */ }
        }
        preloadCache.current.set(blobId, {
          url, filename: idbCached.filename, mimeType: idbCached.mimeType,
          mediaType: resolvedType, cropData: photoCropData,
          isFavorite: idbCached.isFavorite ?? false,
        });
        setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
        setLoading(false);
        return;
      }

      // Cache miss — download, decrypt, display
      const encrypted = await api.blobs.download(blobId);
      const decrypted = await decrypt(encrypted);
      const payload: MediaPayload = JSON.parse(new TextDecoder().decode(decrypted));

      setFilename(payload.filename);
      setMimeType(payload.mime_type);

      // Derive media type from payload, then MIME, then default to photo
      const resolvedType: MediaType =
        payload.media_type ??
        (payload.mime_type === "image/gif"
          ? "gif"
          : payload.mime_type.startsWith("video/")
          ? "video"
          : payload.mime_type.startsWith("audio/")
          ? "audio"
          : "photo");
      setMediaType(resolvedType);

      // Decode base64 → Blob → Object URL
      const bytes = base64ToUint8Array(payload.data).buffer as ArrayBuffer;
      const blob = new Blob([bytes], { type: payload.mime_type });
      const url = URL.createObjectURL(blob);
      setMediaUrl(url);

      // Load crop data from IndexedDB for cache entry
      let photoCropData = null;
      const dbEntry = await db.photos.get(blobId);
      if (dbEntry?.cropData) {
        try { photoCropData = JSON.parse(dbEntry.cropData); } catch { /* ignore */ }
      }

      // Store in preload cache so swiping back is instant
      preloadCache.current.set(blobId, {
        url,
        filename: payload.filename,
        mimeType: payload.mime_type,
        mediaType: resolvedType,
        cropData: photoCropData,
        isFavorite: false,
      });

      // Cache decrypted data in IndexedDB for cross-session persistence
      if (blob.size < 50 * 1024 * 1024) {
        try {
          await db.fullPhotos?.put({
            photoId: blobId, filename: payload.filename, mimeType: payload.mime_type,
            mediaType: resolvedType, cropData: dbEntry?.cropData ?? undefined,
            isFavorite: false, data: bytes, cachedAt: Date.now(),
          });
        } catch { /* non-fatal */ }
      }

      // Revoke the preview now that full media is ready
      setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load media");
    } finally {
      setLoading(false);
    }
  }, [preloadCache]);

  return {
    mediaUrl, setMediaUrl,
    previewUrl, setPreviewUrl,
    filename, setFilename,
    mimeType, setMimeType,
    mediaType, setMediaType,
    loading, setLoading,
    error, setError,
    videoError, setVideoError,
    isConverting, setIsConverting,
    loadPlainMedia,
    loadEncryptedMedia,
    preloadCacheRef: preloadCache,
  };
}
