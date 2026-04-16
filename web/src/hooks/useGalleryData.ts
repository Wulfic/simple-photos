/**
 * Hook for loading and managing gallery data in encrypted mode.
 *
 * Thin orchestrator that composes:
 *  - useSecureBlobFilter — tracks which blob IDs are in secure galleries
 *  - usePhotoSync        — synchronises server→IDB + periodic re-sync
 */
import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { hasCryptoKey } from "../crypto/crypto";
import type { CachedPhoto } from "../db";
import { useSecureBlobFilter } from "../gallery/hooks/useSecureBlobFilter";
import { usePhotoSync } from "../gallery/hooks/usePhotoSync";
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
  const navigate = useNavigate();
  const [error, setError] = useState("");
  const { secureBlobIds, refreshSecureBlobIds, startPolling } = useSecureBlobFilter();
  const { encryptedPhotos, loading, loadEncryptedPhotos } = usePhotoSync();

  useEffect(() => {
    async function init() {
      try {
        await refreshSecureBlobIds();
        startPolling();

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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return {
    loading,
    error,
    setError,
    encryptedPhotos,
    secureBlobIds,
    loadEncryptedPhotos,
  };
}
