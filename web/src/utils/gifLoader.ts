/**
 * Load the full animated GIF file for gallery tile display.
 *
 * For encrypted GIFs ≤5MB the thumbnail itself is already animated
 * (mime=image/gif) and this function is not needed.  It is called only
 * for large encrypted GIFs whose thumbnail is a static JPEG frame, or
 * for server-side photos accessed via the photos API.
 *
 * Checks the IndexedDB full-photo cache first, then downloads + decrypts
 * the encrypted blob.  Returns an object URL for the decoded GIF, or null
 * if the load fails (caller should keep showing the thumbnail).
 */
import { db } from "../db";
import { api } from "../api/client";
import { decryptPhotoBlobToBlob } from "../crypto/blobEnvelope";
import { useAuthStore } from "../store/auth";

/** In-flight promises keyed by blobId to avoid duplicate downloads. */
const inflight = new Map<string, Promise<string | null>>();

export async function loadFullGif(blobId: string, serverPhotoId?: string): Promise<string | null> {
  // De-duplicate concurrent requests for the same blob
  const existing = inflight.get(blobId);
  if (existing) return existing;

  const promise = _load(blobId, serverPhotoId).finally(() => inflight.delete(blobId));
  inflight.set(blobId, promise);
  return promise;
}

/** Maximum time (ms) to wait for a full GIF download before giving up. */
const GIF_LOAD_TIMEOUT_MS = 30_000;

async function _load(blobId: string, serverPhotoId?: string): Promise<string | null> {
  try {
    // 1. Check IndexedDB full-photo cache
    const cached = await db.fullPhotos?.get(blobId);
    if (cached?.data) {
      return URL.createObjectURL(new Blob([cached.data], { type: cached.mimeType }));
    }

    // Enforce a timeout on network fetches
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), GIF_LOAD_TIMEOUT_MS);

    try {
      // 2. Server-side (unencrypted) photo — load via photos API
      if (serverPhotoId) {
        const token = useAuthStore.getState().accessToken;
        const res = await fetch(`/api/photos/${serverPhotoId}/file`, {
          headers: {
            "Authorization": `Bearer ${token}`,
            "X-Requested-With": "SimplePhotos",
          },
          signal: controller.signal,
        });
        if (!res.ok) return null;
        const blob = await res.blob();
        return URL.createObjectURL(blob);
      }

      // 3. Encrypted blob — download + decrypt (handles v1 + v2 chunked; the
      // Blob is built from per-chunk parts, no giant contiguous array).
      const encrypted = await api.blobs.download(blobId, controller.signal);
      const { blob } = await decryptPhotoBlobToBlob(encrypted, "image/gif");
      return URL.createObjectURL(blob);
    } finally {
      clearTimeout(timer);
    }
  } catch (err) {
    console.warn(`[GIF_LOADER] Failed to load full GIF ${blobId}:`, err); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
    return null;
  }
}
