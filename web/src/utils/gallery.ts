/**
 * Gallery utilities — thumbnail generation, in-memory thumbnail caching,
 * and cursor-based pagination helpers.
 */
import { api } from "../api/client";

// ── Helpers ───────────────────────────────────────────────────────────────────

/**
 * Generate a cover-cropped JPEG thumbnail from an image file.
 * Draws the image scaled to fill a square canvas, center-cropped.
 * @param file - Source image file
 * @param size - Thumbnail dimension in pixels (square)
 * @returns JPEG ArrayBuffer at 80% quality
 */
export function generateImageThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    const url = URL.createObjectURL(file);
    img.onload = () => {
      URL.revokeObjectURL(url);
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const ctx = canvas.getContext("2d")!;
      // Cover-crop: fill the square
      const scale = Math.max(size / img.width, size / img.height);
      const w = img.width * scale;
      const h = img.height * scale;
      ctx.drawImage(img, (size - w) / 2, (size - h) / 2, w, h);
      canvas.toBlob(
        (blob) => (blob ? blob.arrayBuffer().then(resolve) : reject(new Error("Canvas toBlob failed"))),
        "image/jpeg",
        0.8
      );
    };
    img.onerror = () => { URL.revokeObjectURL(url); reject(new Error("Image load failed")); };
    img.src = url;
  });
}

/** Seek to 10 % of video duration and capture a frame. */
export function generateVideoThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const video = document.createElement("video");
    video.muted = true;
    video.playsInline = true;
    const url = URL.createObjectURL(file);

    video.onloadedmetadata = () => {
      // Seek to 10 % of the video (at least 1 s in)
      video.currentTime = Math.min(Math.max(video.duration * 0.1, 1), video.duration);
    };

    video.onseeked = () => {
      URL.revokeObjectURL(url);
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const ctx = canvas.getContext("2d")!;
      const scale = Math.max(size / video.videoWidth, size / video.videoHeight);
      const w = video.videoWidth * scale;
      const h = video.videoHeight * scale;
      ctx.drawImage(video, (size - w) / 2, (size - h) / 2, w, h);
      canvas.toBlob(
        (blob) => (blob ? blob.arrayBuffer().then(resolve) : reject(new Error("Canvas toBlob failed"))),
        "image/jpeg",
        0.8
      );
    };

    video.onerror = () => { URL.revokeObjectURL(url); reject(new Error("Video load failed")); };
    video.src = url;
  });
}

/** Generate a JPEG thumbnail from any image or video file.
 *  For GIFs, returns scaled animated GIF data (preserving animation).
 *  For videos, captures a frame at 10% of duration.
 *  For everything else, returns a JPEG cover-crop. */
export async function generateThumbnail(file: File, size: number): Promise<{ data: ArrayBuffer; mimeType: string }> {
  if (file.type.startsWith("video/")) {
    return { data: await generateVideoThumbnail(file, size), mimeType: "image/jpeg" };
  }
  if (file.type.startsWith("audio/")) {
    // Audio files have no visual content; return a small placeholder
    const fallback = await generateImageThumbnail(new File([new Blob()], file.name, { type: "image/png" }), size).catch(() => new ArrayBuffer(0));
    return { data: fallback, mimeType: "image/jpeg" };
  }
  if (file.type === "image/gif") {
    // Preserve GIF animation by using the original file data as the thumbnail.
    // For large GIFs (>5 MB) fall back to static first-frame JPEG to save space.
    const MAX_GIF_THUMB_BYTES = 5 * 1024 * 1024;
    if (file.size <= MAX_GIF_THUMB_BYTES) {
      return { data: await file.arrayBuffer(), mimeType: "image/gif" };
    }
    // Large GIF — static first-frame is the safer default
    return { data: await generateImageThumbnail(file, size), mimeType: "image/jpeg" };
  }
  return { data: await generateImageThumbnail(file, size), mimeType: "image/jpeg" };
}

/** Return a data URL to preview a thumbnail stored as ArrayBuffer.
 *  @param mimeType - Thumbnail MIME type (defaults to "image/jpeg") */
export function thumbnailSrc(data: ArrayBuffer, mimeType?: string): string {
  return URL.createObjectURL(new Blob([data], { type: mimeType || "image/jpeg" }));
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
