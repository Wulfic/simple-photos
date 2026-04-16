/**
 * GIF detection and classification helpers.
 *
 * Centralises the scattered `mediaType === "gif"` / `mime === "image/gif"`
 * checks used by ThumbnailTile, Search, usePhotoPreload, and useViewerMedia.
 */

import { GIF_THUMB_MAX_BYTES } from "./thumbnailGenerate";

export { GIF_THUMB_MAX_BYTES };

/** True when the MIME type indicates a GIF image. */
export function isGifMime(mimeType: string): boolean {
  return mimeType === "image/gif";
}

/**
 * True when the thumbnail itself was stored as an animated GIF
 * (i.e. the original was ≤ 5 MB and kept as-is).
 */
export function isAnimatedGifThumbnail(thumbnailMimeType?: string | null): boolean {
  return thumbnailMimeType === "image/gif";
}

/**
 * True when the GIF needs full-blob loading for autoplay because the
 * stored thumbnail is a static JPEG first-frame (original > 5 MB).
 */
export function needsFullGifLoad(
  mediaType: string,
  thumbnailMimeType?: string | null,
): boolean {
  return mediaType === "gif" && !isAnimatedGifThumbnail(thumbnailMimeType);
}

/**
 * Derive the gallery `mediaType` field from a raw MIME string.
 * Returns `"gif"` for GIFs, `"video"` for video/*, `"audio"` for audio/*,
 * and `"photo"` for everything else.
 */
export function mediaTypeFromMime(mimeType: string): "photo" | "gif" | "video" | "audio" {
  if (mimeType === "image/gif") return "gif";
  if (mimeType.startsWith("video/")) return "video";
  if (mimeType.startsWith("audio/")) return "audio";
  return "photo";
}
