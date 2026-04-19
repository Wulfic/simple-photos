/**
 * Gallery engine type definitions.
 *
 * These types form the contract between the thumbnail loading pipeline,
 * the cache layer, and the tile display components.
 */

/** All the information needed to resolve a photo's thumbnail. */
export interface ThumbnailSource {
  /** Primary blob ID (also used as IDB key) */
  blobId: string;
  /** Actual storage blob ID (may differ from blobId for copies) */
  storageBlobId?: string;
  /** Encrypted thumbnail blob ID (for encrypted-mode photos) */
  encryptedThumbBlobId?: string;
  /** Server-side photo record ID (for autoscanned / server-side photos) */
  serverPhotoId?: string;
  /** True when the photo is unencrypted on the server */
  serverSide?: boolean;
  /** Pre-loaded thumbnail data from IDB (avoids redundant lookups) */
  thumbnailData?: ArrayBuffer;
  /** MIME type of the thumbnail data */
  thumbnailMimeType?: string;
}

/** Loading state for a thumbnail. */
export type ThumbnailState = "loading" | "cached" | "error" | "placeholder";

/** Result of the thumbnail loading pipeline. */
export interface ThumbnailResult {
  /** Object URL or data URL to display */
  url: string | null;
  /** MIME type of the resolved thumbnail */
  mimeType: string;
  /** Current loading state */
  state: ThumbnailState;
  /** Retry loading (e.g. after a transient error) */
  retry: () => void;
}

/** Media badge shown on a tile corner. */
export type MediaBadge = "video" | "gif" | "audio" | null;

/** Props for the unified ThumbnailTile component. */
export interface ThumbnailTileProps {
  /** Thumbnail resolution data */
  source: ThumbnailSource;
  /** Media type — drives badge and GIF autoplay */
  mediaType: "photo" | "gif" | "video" | "audio";
  /** Original filename (for alt text and audio overlay) */
  filename: string;
  /** Crop/edit metadata JSON string */
  cropData?: string | null;
  /** Video/audio duration in seconds */
  duration?: number;
  /** Photo subtype: "burst", "motion", "panorama", "equirectangular", "hdr" */
  photoSubtype?: string;
  /** Number of photos in a burst stack (only set on the representative frame) */
  burstCount?: number;
  /** Tile click handler */
  onClick: () => void;
  /** Long-press handler (selection mode) */
  onLongPress?: () => void;
  /** Whether the gallery is in selection mode */
  selectionMode?: boolean;
  /** Whether this tile is selected */
  isSelected?: boolean;
  /** Optional dimension self-heal callback */
  onDimensionMismatch?: (correctedWidth: number, correctedHeight: number) => void;
}
