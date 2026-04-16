/**
 * Gallery engine — public API.
 *
 * Re-exports all types, hooks, cache utilities, and components that
 * consumers (pages, other modules) should use.
 */

// Types
export type {
  ThumbnailSource,
  ThumbnailState,
  ThumbnailResult,
  MediaBadge,
  ThumbnailTileProps,
} from "./types";

// Cache layer
export { thumbnailCache } from "./cache/thumbnailCache";
export { blobUrlManager } from "./cache/blobUrlManager";

// Hooks
export { useThumbnailLoader } from "./hooks/useThumbnailLoader";
export { useGifAutoplay } from "./hooks/useGifAutoplay";
export { useSecureItemSource } from "./hooks/useSecureItemSource";
export { useSecureBlobFilter } from "./hooks/useSecureBlobFilter";
export { usePhotoSync } from "./hooks/usePhotoSync";
export {
  detectOrientationSwap,
  isTransposed,
  correctDimensionsFromThumbnail,
  queueDimensionUpdate,
  applyDimensionCorrection,
} from "./hooks/useDimensionSync";
export type { GifAutoplayState, GifAutoplayResult } from "./hooks/useGifAutoplay";

// Components
export { default as ThumbnailTile } from "./components/ThumbnailTile";
export { default as PickerThumbnail } from "./components/PickerThumbnail";
export { default as SecureGalleryItem } from "./components/SecureGalleryItem";

// Thumbnail generation & GIF detection
export {
  generateThumbnail,
  decodeThumbnailDimensions,
  thumbnailSrc,
  GIF_THUMB_MAX_BYTES,
} from "./utils/thumbnailGenerate";
export type { ThumbnailOptions } from "./utils/thumbnailGenerate";
export {
  isGifMime,
  isAnimatedGifThumbnail,
  needsFullGifLoad,
  mediaTypeFromMime,
} from "./utils/gifDetection";
