/**
 * Unified thumbnail loading hook — replaces the independent loading logic
 * in MediaTile, SecureGalleryTile.ItemTile, and PhotoThumbnail.
 *
 * Fallback chain (single implementation):
 *  1. Unified cache (hit → return immediately)
 *  2. IDB `thumbs` table → create blob URL → cache
 *  3. Encrypted thumb blob download → decrypt → blob URL → cache
 *  4. Server API fallback `/api/photos/{id}/thumbnail`
 *
 * Step 2 reads the cache itself rather than taking bytes through
 * `ThumbnailSource`. Callers used to pass `photo.thumbnailData` straight from a
 * mirror row, which is exactly what forced every list rendering tiles to hydrate
 * every thumbnail in the library up-front. Now they pass ids, and each tile
 * fetches only its own bytes, only once it's on screen.
 *
 * Blob URLs are created by `thumbnailCache` and by nothing else (#51). This hook
 * previously minted them through `blobUrlManager` while the cache revoked them
 * directly — two owners, and the one revoking was not the one counting refs, so
 * an evicted thumbnail's URL stayed in the manager and was handed back dead on
 * every subsequent load. While a tile is mounted it holds a *pin*, which makes
 * it ineligible for eviction; the pin is released on unmount or when the tile
 * switches to a different blob.
 *
 * Returns `{ url, mimeType, state, retry }`.
 */
import { useState, useEffect, useRef, useCallback } from "react";
import { thumbnailCache } from "../cache/thumbnailCache";
import { blobsApi } from "../../api/blobs";
import { getThumb } from "../../db/thumbs";
import { decryptPhotoBlob } from "../../crypto/blobEnvelope";
import { useAuthStore } from "../../store/auth";
import { appendGalleryTokenParam } from "../../utils/galleryToken";
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
  /** blobId this tile currently holds a cache pin on, if any. */
  const pinnedRef = useRef<string | null>(null);

  const resolve = useCallback((resolvedUrl: string, mime: string) => {
    if (!mountedRef.current) return;
    setUrl(resolvedUrl);
    setMimeType(mime);
    setState("cached");
  }, []);

  /** Release whatever this tile was pinning. Safe to call repeatedly. */
  const releasePin = useCallback(() => {
    if (pinnedRef.current === null) return;
    thumbnailCache.unpin(pinnedRef.current);
    pinnedRef.current = null;
  }, []);

  /**
   * Pin the blob this tile is about to display, so eviction cannot revoke the
   * URL out from under a mounted <img>. Re-pinning the same id is a no-op.
   */
  const holdPin = useCallback((blobId: string) => {
    if (pinnedRef.current === blobId) return;
    releasePin();
    thumbnailCache.pin(blobId);
    pinnedRef.current = blobId;
  }, [releasePin]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      releasePin();
    };
  }, [releasePin]);

  useEffect(() => {
    if (!enabled) {
      releasePin();
      setState("placeholder");
      setUrl(null);
      return;
    }

    const { blobId } = source;

    // 1. Check unified cache
    const cached = thumbnailCache.get(blobId);
    if (cached) {
      holdPin(blobId);
      resolve(cached.url, cached.mimeType);
      return;
    }

    // 3. Server-side photo — use server thumbnail API directly.
    //    A plain HTTP URL, not a blob URL, so there is nothing to pin.
    if (source.serverSide && source.serverPhotoId) {
      releasePin();
      const token = useAuthStore.getState().accessToken;
      const serverUrl = appendGalleryTokenParam(
        `/api/photos/${source.serverPhotoId}/thumbnail?token=${encodeURIComponent(token || "")}`,
      );
      resolve(serverUrl, "image/jpeg");
      return;
    }

    setState("loading");
    let cancelled = false;
    (async () => {
      // 2. Cached thumbnail bytes (thumbs table, or a not-yet-backfilled row).
      try {
        const cachedThumb = await getThumb(blobId);
        if (cancelled) return;
        if (cachedThumb && cachedThumb.data.byteLength > 0) {
          const mime = source.thumbnailMimeType || cachedThumb.mime;
          const { url: thumbUrl } = thumbnailCache.getOrCreate(blobId, cachedThumb.data, mime);
          holdPin(blobId);
          resolve(thumbUrl, mime);
          return;
        }
      } catch (err) {
        // Fall through to the network paths — a cache read failing is not fatal.
        console.warn(`[THUMB_LOADER] IDB thumb lookup failed for ${blobId}:`, err); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
        if (cancelled) return;
      }

      // 4. Encrypted thumbnail blob — download + decrypt
      if (source.encryptedThumbBlobId) {
        try {
          const encData = await blobsApi.download(source.encryptedThumbBlobId);
          if (cancelled) return;
          const { payload, bytes } = await decryptPhotoBlob(encData);
          if (cancelled) return;
          if (!bytes.byteLength) throw new Error("No data in encrypted thumbnail payload");
          const mime = payload.mime_type || "image/jpeg";
          const { url: thumbUrl } = thumbnailCache.getOrCreate(
            blobId,
            bytes.buffer as ArrayBuffer,
            mime,
          );
          if (!cancelled) {
            holdPin(blobId);
            resolve(thumbUrl, mime);
          }
        } catch (err) {
          console.warn(`[THUMB_LOADER] Encrypted thumb download failed for ${blobId}:`, err); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
          if (!cancelled) _tryServerFallback();
        }
        return;
      }

      // 5. No thumbnail data available yet — show placeholder
      if (!cancelled) {
        setState("placeholder");
        setUrl(null);
      }
    })();
    return () => { cancelled = true; };

    function _tryServerFallback() {
      // Last resort: try the server photos API directly. An HTTP URL, so any
      // pin this tile held no longer describes what it is displaying.
      releasePin();
      const token = useAuthStore.getState().accessToken;
      if (token && blobId) {
        const serverUrl = appendGalleryTokenParam(
          `/api/photos/${blobId}/thumbnail?token=${encodeURIComponent(token)}`,
        );
        resolve(serverUrl, "image/jpeg");
      } else {
        setState("error");
      }
    }
  }, [
    enabled,
    source.blobId,
    source.thumbnailMimeType,
    source.serverSide,
    source.serverPhotoId,
    source.encryptedThumbBlobId,
    retryCount,
    resolve,
    holdPin,
    releasePin,
  ]);

  const retry = useCallback(() => {
    setRetryCount((c) => c + 1);
    setState("loading");
  }, []);

  return { url, mimeType, state, retry };
}
