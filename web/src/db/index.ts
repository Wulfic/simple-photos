import Dexie, { type Table } from "dexie";

/** Discriminated union for the three media categories */
export type MediaType = "photo" | "gif" | "video";

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
  /** Duration in seconds for video blobs (undefined for photos/GIFs) */
  duration?: number;
  /** Crop/edit metadata (JSON string) — used for encrypted-mode crops stored locally */
  cropData?: string;
}

export interface CachedAlbum {
  albumId: string;
  manifestBlobId: string;
  name: string;
  createdAt: number;
  coverPhotoBlobId?: string;
  photoBlobIds: string[];
}

/** A locally cached trash item (encrypted mode only).
 *  Plain-mode trash is managed entirely server-side. */
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

class SimplePhotosDB extends Dexie {
  photos!: Table<CachedPhoto, string>;
  albums!: Table<CachedAlbum, string>;
  trash!: Table<CachedTrashItem, string>;

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
  }
}

export const db = new SimplePhotosDB();

/** Derive the server blob type from a MIME type string */
export function blobTypeFromMime(mimeType: string): string {
  if (mimeType === "image/gif") return "gif";
  if (mimeType.startsWith("video/")) return "video";
  return "photo";
}

/** Derive the MediaType from a MIME type string */
export function mediaTypeFromMime(mimeType: string): MediaType {
  if (mimeType === "image/gif") return "gif";
  if (mimeType.startsWith("video/")) return "video";
  return "photo";
}

/**
 * All accepted MIME types for the file picker.
 * The server stores blobs opaquely, so every format is valid as long as
 * the browser can read it for thumbnail generation.
 */
export const ACCEPTED_MIME_TYPES = [
  // ── Images ──────────────────────────────────────────────────────────────────
  "image/*",
  // ── Videos ──────────────────────────────────────────────────────────────────
  "video/*",
].join(",");
