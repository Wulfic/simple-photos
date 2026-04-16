/**
 * Unified thumbnail loading hook — replaces the independent loading logic
 * in MediaTile, SecureGalleryTile.ItemTile, and PhotoThumbnail.
 *
 * Fallback chain (single implementation):
 *  1. Unified cache (hit → return immediately)
 *  2. IDB `photo.thumbnailData` → create blob URL → cache
 *  3. Encrypted thumb blob download → decrypt → blob URL → cache
 *  4. Server API fallback `/api/photos/{id}/thumbnail`
 *
 * Returns `{ url, mimeType, state, retry }`.
 */
import { useState, useEffect, useRef, useCallback } from "react";
import { thumbnailCache } from "../cache/thumbnailCache";
import { blobUrlManager } from "../cache/blobUrlManager";
import { blobsApi } from "../../api/blobs";
import { decrypt } from "../../crypto/crypto";
import { useAuthStore } from "../../store/auth";
import type { ThumbnailSource, ThumbnailState, ThumbnailResult } from "../types";

export function useThumbnailLoader(
  source: ThumbnailSource,
  enabled: boolean = true,
): ThumbnailResult {
  const [state, setState] = useState<ThumbnailState>("loading");
  const [url, setUrl] = useState<string | null>(null);
  const [mimeType, setMimeType] = useState("image/jpeg");
  const [retryCount, setRetryCount] = useState(0);
  const mountedRef = useRef(true);

  const resolve = useCallback((resolvedUrl: string, mime: string) => {
    if (!mountedRef.current) return;
    setUrl(resolvedUrl);
    setMimeType(mime);
    setState("cached");
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  useEffect(() => {
    if (!enabled) {
      setState("placeholder");
      setUrl(null);
      return;
    }

    const { blobId } = source;

    // 1. Check unified cache
    const cached = thumbnailCache.get(blobId);
    if (cached) {
      resolve(cached.url, cached.mimeType);
      return;
    }

    // 2. IDB thumbnail data (passed in via source)
    if (source.thumbnailData && source.thumbnailData.byteLength > 0) {
      const mime = source.thumbnailMimeType || "image/jpeg";
      const thumbUrl = blobUrlManager.acquire(
        `thumb:${blobId}`,
        source.thumbnailData,
        mime,
      );
      thumbnailCache.set(blobId, thumbUrl, mime);
      resolve(thumbUrl, mime);
      return;
    }

    // 3. Server-side photo — use server thumbnail API directly
    if (source.serverSide && source.serverPhotoId) {
      const token = useAuthStore.getState().accessToken;
      const serverUrl = `/api/photos/${source.serverPhotoId}/thumbnail?token=${encodeURIComponent(token || "")}`;
      resolve(serverUrl, "image/jpeg");
      return;
    }

    // 4. Encrypted thumbnail blob — download + decrypt
    if (source.encryptedThumbBlobId) {
      setState("loading");
      let cancelled = false;
      (async () => {
        try {
          const encData = await blobsApi.download(source.encryptedThumbBlobId!);
          if (cancelled) return;
          const plaintext = await decrypt(encData);
          if (cancelled) return;
          const json = JSON.parse(new TextDecoder().decode(plaintext));
          const b64 = json.data as string;
          if (!b64) throw new Error("No data in encrypted thumbnail payload");
          const binary = atob(b64);
          const bytes = new Uint8Array(binary.length);
          for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
          const mime = json.mime_type || "image/jpeg";
          const thumbUrl = blobUrlManager.acquire(
            `thumb:${blobId}`,
            bytes.buffer as ArrayBuffer,
            mime,
          );
          thumbnailCache.set(blobId, thumbUrl, mime);
          if (!cancelled) resolve(thumbUrl, mime);
        } catch (err) {
          console.warn(`[THUMB_LOADER] Encrypted thumb download failed for ${blobId}:`, err);
          if (!cancelled) _tryServerFallback();
        }
      })();
      return () => { cancelled = true; };
    }

    // 5. No thumbnail data available yet — show placeholder
    setState("placeholder");
    setUrl(null);

    function _tryServerFallback() {
      // Last resort: try the server photos API directly
      const token = useAuthStore.getState().accessToken;
      if (token && blobId) {
        const serverUrl = `/api/photos/${blobId}/thumbnail?token=${encodeURIComponent(token)}`;
        resolve(serverUrl, "image/jpeg");
      } else {
        setState("error");
      }
    }
  }, [
    enabled,
    source.blobId,
    source.thumbnailData,
    source.thumbnailMimeType,
    source.serverSide,
    source.serverPhotoId,
    source.encryptedThumbBlobId,
    retryCount,
    resolve,
  ]);

  const retry = useCallback(() => {
    setRetryCount((c) => c + 1);
    setState("loading");
  }, []);

  return { url, mimeType, state, retry };
}
