/**
 * Hook that encapsulates media loading logic for the Viewer page.
 *
 * Handles encrypted-mode (download + decrypt) media loading, with IndexedDB
 * caching for instant display on return visits and cross-session persistence.
 */
import { useState, useCallback, useRef } from "react";
import { api } from "../api/client";
import { decrypt } from "../crypto/crypto";
import { db, type MediaType } from "../db";
import { base64ToUint8Array } from "../utils/media";
import type { MediaPayload, PreloadEntry, CropMetadata, PhotoInfoData } from "../types/media";
export type { MediaPayload, PreloadEntry, CropMetadata, PhotoInfoData };

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
  loadEncryptedMedia: (blobId: string) => Promise<void>;
  preloadCacheRef: React.MutableRefObject<Map<string, PreloadEntry>>;
}

export default function useViewerMedia(
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
  const abortRef = useRef<AbortController | null>(null);

  /** Load media — check IndexedDB cache first, then download + decrypt. */
  const loadEncryptedMedia = useCallback(async (blobId: string) => {
    // Abort any in-flight download before starting a new one
    if (abortRef.current) {
      abortRef.current.abort();
    }
    const controller = new AbortController();
    abortRef.current = controller;

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
      console.log(`[DIAG:VIEWER] Downloading blob ${blobId}...`);
      const encrypted = await api.blobs.download(blobId, controller.signal);
      // Check if aborted during download
      if (controller.signal.aborted) return;
      console.log(`[DIAG:VIEWER] Downloaded ${encrypted.byteLength} bytes, decrypting...`);
      const decrypted = await decrypt(encrypted);
      if (controller.signal.aborted) return;
      console.log(`[DIAG:VIEWER] Decrypted ${decrypted.byteLength} bytes, parsing JSON...`);
      const payload: MediaPayload = JSON.parse(new TextDecoder().decode(decrypted));
      console.log(`[DIAG:VIEWER] Payload: mime_type=${payload.mime_type}, media_type=${payload.media_type}, filename=${payload.filename}, data_length=${payload.data?.length ?? 0}`);

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
      console.log(`[DIAG:VIEWER] Resolved mediaType=${resolvedType}`);

      // Decode base64 → Blob → Object URL
      const bytes = base64ToUint8Array(payload.data).buffer as ArrayBuffer;
      const blob = new Blob([bytes], { type: payload.mime_type });
      const url = URL.createObjectURL(blob);
      console.log(`[DIAG:VIEWER] Created blob URL: type=${payload.mime_type}, size=${blob.size}`);
      setMediaUrl(url);

      // Load crop data from IndexedDB for cache entry
      let photoCropData = null;
      const cachedEntry = await db.photos.get(blobId);
      if (cachedEntry?.cropData) {
        try { photoCropData = JSON.parse(cachedEntry.cropData); } catch { /* ignore */ }
      }

      // Read favorite status from the CachedPhoto entry (synced from server)
      const photoIsFavorite = cachedEntry?.isFavorite ?? false;

      // Store in preload cache so swiping back is instant
      preloadCache.current.set(blobId, {
        url,
        filename: payload.filename,
        mimeType: payload.mime_type,
        mediaType: resolvedType,
        cropData: photoCropData,
        isFavorite: photoIsFavorite,
      });

      // Cache decrypted data in IndexedDB for cross-session persistence
      if (blob.size < 50 * 1024 * 1024) {
        try {
          await db.fullPhotos?.put({
            photoId: blobId, filename: payload.filename, mimeType: payload.mime_type,
            mediaType: resolvedType, cropData: cachedEntry?.cropData ?? undefined,
            isFavorite: photoIsFavorite, data: bytes, cachedAt: Date.now(),
          });
        } catch { /* non-fatal */ }
      }

      // Revoke the preview now that full media is ready
      setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
    } catch (err: unknown) {
      // Silently ignore aborted downloads (user navigated away)
      if (err instanceof DOMException && err.name === "AbortError") return;
      setError(err instanceof Error ? err.message : "Failed to load media");
    } finally {
      if (!controller.signal.aborted) {
        setLoading(false);
      }
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
    loadEncryptedMedia,
    preloadCacheRef: preloadCache,
  };
}
