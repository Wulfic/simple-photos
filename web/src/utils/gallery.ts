/**
 * Gallery utilities — cursor-based pagination helpers, caching, duration
 * formatting, and legacy re-exports of functions now living in the
 * gallery module.
 */
import { api } from "../api/client";

// ── Re-exports from gallery module (kept for backward compatibility) ──────────
export {
  generateThumbnail,
  decodeThumbnailDimensions,
  thumbnailSrc,
} from "../gallery/utils/thumbnailGenerate";

// ── Legacy generation functions ───────────────────────────────────────────────
// These thin wrappers delegate to the unified `generateThumbnail`.
// New code should import from `../gallery` directly.

import { generateThumbnail as _generate } from "../gallery/utils/thumbnailGenerate";
import { createFallbackThumbnail } from "./media";

/**
 * @deprecated Use `generateThumbnail(file, { size })` from `../gallery` instead.
 */
export async function generateImageThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  const result = await _generate(file, { size });
  return result.data;
}

/**
 * @deprecated Use `generateThumbnail(file, { size })` from `../gallery` instead.
 */
export async function generateVideoThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  const result = await _generate(file, { size });
  return result.data;
}

/** Get the natural width/height of an image file. */
export function getImageDimensions(file: File): Promise<{ width: number; height: number }> {
  return new Promise((resolve) => {
    if (file.type.startsWith("audio/")) {
      // Audio files have no visual dimensions
      resolve({ width: 0, height: 0 });
    } else if (file.type.startsWith("video/")) {
      const video = document.createElement("video");
      const url = URL.createObjectURL(file);
      video.onloadedmetadata = () => {
        URL.revokeObjectURL(url);
        resolve({ width: video.videoWidth, height: video.videoHeight });
      };
      video.onerror = () => { URL.revokeObjectURL(url); resolve({ width: 0, height: 0 }); };
      video.src = url;
    } else {
      const img = new Image();
      const url = URL.createObjectURL(file);
      img.onload = () => { URL.revokeObjectURL(url); resolve({ width: img.naturalWidth, height: img.naturalHeight }); };
      img.onerror = () => { URL.revokeObjectURL(url); resolve({ width: 0, height: 0 }); };
      img.src = url;
    }
  });
}

// ── Paginated blob fetching ───────────────────────────────────────────────────

/** Fetch all pages of a given blob type from the server. */
export async function fetchAllPages(blobType: string) {
  const allBlobs: Array<{
    id: string;
    blob_type: string;
    size_bytes: number;
    client_hash: string | null;
    upload_time: string;
    content_hash: string | null;
  }> = [];
  let cursor: string | undefined;
  do {
    const res = await api.blobs.list({
      blob_type: blobType,
      after: cursor,
      limit: 200,
    });
    allBlobs.push(...res.blobs);
    cursor = res.next_cursor ?? undefined;
  } while (cursor);
  return allBlobs;
}

/** Format a duration in seconds as `M:SS` (e.g. `2:05`). */
export function formatDuration(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

// ── Thumbnail Cache ───────────────────────────────────────────────────────────
// Uses the browser's Cache API to persistently cache thumbnails across loads.
// Falls back to in-memory Map if Cache API is unavailable.

export const THUMB_CACHE_NAME = "sp-thumbnails-v1";
export const thumbMemoryCache = new Map<string, string>(); // photoId → objectURL

/**
 * Retrieve a cached thumbnail URL for a photo.
 * Checks in-memory Map first, then the persistent Cache API.
 * @returns An ObjectURL for the thumbnail, or `null` if not cached.
 */
export async function getCachedThumbnail(photoId: string): Promise<string | null> {
  // Check memory cache first (fastest)
  const memUrl = thumbMemoryCache.get(photoId);
  if (memUrl) return memUrl;

  // Check persistent Cache API
  try {
    const cache = await caches.open(THUMB_CACHE_NAME);
    const cacheKey = `/thumb-cache/${photoId}`;
    const cached = await cache.match(cacheKey);
    if (cached) {
      const blob = await cached.blob();
      const url = URL.createObjectURL(blob);
      thumbMemoryCache.set(photoId, url);
      return url;
    }
  } catch {
    // Cache API unavailable — continue to fetch
  }
  return null;
}

/**
 * Store a thumbnail blob in both the in-memory cache and the
 * persistent Cache API (if available).
 * @returns An ObjectURL referencing the cached blob.
 */
export async function cacheThumbnail(photoId: string, blob: Blob): Promise<string> {
  const url = URL.createObjectURL(blob);
  thumbMemoryCache.set(photoId, url);

  // Persist to Cache API
  try {
    const cache = await caches.open(THUMB_CACHE_NAME);
    const cacheKey = `/thumb-cache/${photoId}`;
    const response = new Response(blob, {
      headers: { "Content-Type": blob.type || "image/jpeg" },
    });
    await cache.put(cacheKey, response);
  } catch {
    // Cache API unavailable — memory-only cache is fine
  }

  return url;
}
