/**
 * Dexie (IndexedDB) database schema for client-side caching.
 *
 * Caches decrypted thumbnails, photo metadata, album definitions, and trash
 * items so the UI can render instantly without waiting for server round-trips.
 * Data is per-device and can be safely cleared without data loss.
 */
import Dexie, { type Table } from "dexie";

/** Discriminated union for the four media categories */
export type MediaType = "photo" | "gif" | "video" | "audio";

export interface CachedPhoto {
  blobId: string;
  thumbnailBlobId?: string;
  filename: string;
  takenAt: number;
  /** MIME type of the original file (e.g. "image/jpeg", "video/mp4", "image/gif") */
  mimeType: string;
  /** "photo" | "gif" | "video" — drives which player to use in the Viewer */
  mediaType: MediaType;
  width: number;
  height: number;
  latitude?: number;
  longitude?: number;
  albumIds: string[];
  thumbnailData?: ArrayBuffer;
  /** MIME type of the thumbnail data (e.g. "image/gif" for animated GIF thumbnails).
   *  Defaults to "image/jpeg" when not set (backwards compatibility). */
  thumbnailMimeType?: string;
  /** Duration in seconds for video blobs (undefined for photos/GIFs) */
  duration?: number;
  /** Crop/edit metadata (JSON string) — used for encrypted-mode crops stored locally */
  cropData?: string;
  /** Short content-based hash (12 hex chars of SHA-256) for cross-platform alignment */
  contentHash?: string;
  /** Actual server storage blob ID (defaults to blobId if undefined) */
  storageBlobId?: string;
  /** Whether this photo is marked as favorite — synced with the server */
  isFavorite?: boolean;
  /** Non-null when this photo was converted from a non-native format.
   *  Contains the relative path to the original source file on the server. */
  sourcePath?: string;
  /** Server-side `photos.id` for API calls requiring the photo record ID
   *  (e.g., toggle_favorite, duplicate). Set for photos synced via encrypted-sync. */
  serverPhotoId?: string;
  /** When true, the photo lives on the server as an unencrypted file (autoscan
   *  pipeline). The Viewer should fetch it via the photos API (`/photos/:id/file`)
   *  instead of downloading and decrypting an encrypted blob. */
  serverSide?: boolean;
  /** Photo subtype: "motion", "panorama", "equirectangular", "hdr", "burst" */
  photoSubtype?: string;
  /** Burst group identifier (shared across all shots in a burst sequence) */
  burstId?: string;
  /** Blob ID of the extracted motion video (motion photos only) */
  motionVideoBlobId?: string;
}

export interface CachedAlbum {
  albumId: string;
  manifestBlobId: string;
  name: string;
  createdAt: number;
  coverPhotoBlobId?: string;
  photoBlobIds: string[];
}

/** A locally cached trash item.
 *  Trash is managed with local IndexedDB cache and decrypted thumbnails. */
export interface CachedTrashItem {
  /** Server-assigned trash item ID */
  trashId: string;
  /** Original blob ID */
  blobId: string;
  thumbnailBlobId?: string;
  filename: string;
  mimeType: string;
  mediaType: MediaType;
  width: number;
  height: number;
  takenAt: number;
  deletedAt: number;
  expiresAt: string;
  /** Decrypted thumbnail data for display in the trash view */
  thumbnailData?: ArrayBuffer;
  duration?: number;
  albumIds: string[];
}

/** Cached full-size photo data for instant viewing across sessions.
 *  LRU-evicted: keeps the most recently viewed photos in IndexedDB. */
export interface CachedFullPhoto {
  /** Photo ID or blob ID */
  photoId: string;
  filename: string;
  mimeType: string;
  mediaType: MediaType;
  cropData?: string;
  isFavorite: boolean;
  /** The raw decrypted photo bytes */
  data: ArrayBuffer;
  /** Timestamp when this entry was cached (for LRU eviction) */
  cachedAt: number;
}

/** A metadata-only "copy" of a photo/video/audio.
 *  Copies are stored server-side in the edit_copies table. */
export interface CachedEditCopy {
  /** Unique ID for this copy */
  copyId: string;
  /** The photo/video/audio this copy belongs to */
  photoBlobId: string;
  /** Display name for the copy */
  name: string;
  /** JSON string with edit metadata (crop, brightness, trim, etc.) */
  editMetadata: string;
  /** When this copy was created */
  createdAt: number;
}

class SimplePhotosDB extends Dexie {
  photos!: Table<CachedPhoto, string>;
  albums!: Table<CachedAlbum, string>;
  trash!: Table<CachedTrashItem, string>;
  fullPhotos!: Table<CachedFullPhoto, string>;
  editCopies!: Table<CachedEditCopy, string>;

  constructor() {
    super("simple-photos");

    // v1 — original schema
    this.version(1).stores({
      photos: "blobId, takenAt, *albumIds",
      albums: "albumId, name",
    });

    // v2 — added mediaType index (migration: existing rows get mediaType = "photo")
    this.version(2)
      .stores({
        photos: "blobId, takenAt, mediaType, *albumIds",
        albums: "albumId, name",
      })
      .upgrade((tx) =>
        tx
          .table("photos")
          .toCollection()
          .modify((photo: CachedPhoto) => {
            if (!photo.mediaType) {
              // Infer from mimeType for existing records
              if (photo.mimeType === "image/gif") {
                photo.mediaType = "gif";
              } else if (photo.mimeType?.startsWith("video/")) {
                photo.mediaType = "video";
              } else if (photo.mimeType?.startsWith("audio/")) {
                photo.mediaType = "audio";
              } else {
                photo.mediaType = "photo";
              }
            }
          })
      );

    // v3 — added local trash table for encrypted-mode soft-deletes
    this.version(3).stores({
      photos: "blobId, takenAt, mediaType, *albumIds",
      albums: "albumId, name",
      trash: "trashId, blobId, deletedAt",
    });

    // v4 — added cropData field to photos (no index change needed, just bump version)
    this.version(4).stores({
      photos: "blobId, takenAt, mediaType, *albumIds",
      albums: "albumId, name",
      trash: "trashId, blobId, deletedAt",
    });

    // v5 — added contentHash for cross-platform photo alignment
    this.version(5).stores({
      photos: "blobId, takenAt, mediaType, *albumIds, contentHash",
      albums: "albumId, name",
      trash: "trashId, blobId, deletedAt",
    });

    // v6 — added fullPhotos table for cross-session full-photo caching
    //       LRU-evicted to keep the 200 most recently viewed photos.
    this.version(6).stores({
      photos: "blobId, takenAt, mediaType, *albumIds, contentHash",
      albums: "albumId, name",
      trash: "trashId, blobId, deletedAt",
      fullPhotos: "photoId, cachedAt",
    });

    // v7 — added editCopies table for metadata-only "Save Copy" feature
    this.version(7).stores({
      photos: "blobId, takenAt, mediaType, *albumIds, contentHash",
      albums: "albumId, name",
      trash: "trashId, blobId, deletedAt",
      fullPhotos: "photoId, cachedAt",
      editCopies: "copyId, photoBlobId, createdAt",
    });
  }
}

export const db = new SimplePhotosDB();

/**
 * Wipe ALL user-specific data from local caches.
 *
 * Must be called on logout (and defensively on login) to prevent a
 * flash of the previous user's photos when another account signs in.
 *
 * Clears:
 *  - All 5 IndexedDB tables (photos, albums, trash, fullPhotos, editCopies)
 *  - The Cache API thumbnail cache (sp-thumbnails-v1)
 *
 * The in-memory thumbnail Map (thumbMemoryCache) is cleared separately
 * by the caller since it lives in utils/gallery.ts.
 */
export async function clearAllUserData(): Promise<void> {
  await Promise.all([
    db.photos.clear(),
    db.albums.clear(),
    db.trash.clear(),
    db.fullPhotos.clear(),
    db.editCopies.clear(),
  ]);

  // Wipe persistent thumbnail cache
  try {
    await caches.delete("sp-thumbnails-v1");
  } catch {
    // Cache API may be unavailable — fine
  }
}

/** Derive the server blob type from a MIME type string */
export function blobTypeFromMime(mimeType: string): string {
  if (mimeType === "image/gif") return "gif";
  if (mimeType.startsWith("video/")) return "video";
  if (mimeType.startsWith("audio/")) return "audio";
  return "photo";
}

/** Derive the MediaType from a MIME type string */
export function mediaTypeFromMime(mimeType: string): MediaType {
  if (mimeType === "image/gif") return "gif";
  if (mimeType.startsWith("video/")) return "video";
  if (mimeType.startsWith("audio/")) return "audio";
  return "photo";
}

/**
 * All accepted MIME types for the file picker.
 * The server stores blobs opaquely, so every format is valid as long as
 * the browser can read it for thumbnail generation.
 */
export const ACCEPTED_MIME_TYPES = [
  // ── Images ──────────────────────────────────────────────────────────────────
  "image/jpeg", "image/png", "image/gif", "image/webp", "image/avif", "image/bmp", "image/x-icon",
  // ── Videos ──────────────────────────────────────────────────────────────────
  "video/mp4", "video/webm", "video/quicktime",
  // ── Audio ───────────────────────────────────────────────────────────────────
  "audio/mpeg", "audio/flac", "audio/ogg", "audio/wav",
].join(",");
