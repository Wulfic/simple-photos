/**
 * Shared media type definitions used across hooks and components.
 *
 * These types describe the encrypted payload shapes sent to/from the API
 * and the internal data structures for the viewer and preload cache.
 */
import type { MediaType } from "../db";

// ── Encrypted payload shapes ─────────────────────────────────────────────────

/** Encrypted photo blob payload (full-resolution file). */
export interface MediaPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type?: MediaType;
  width: number;
  height: number;
  duration?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64-encoded raw file bytes
}

/** Encrypted photo metadata + data for gallery upload/sync. */
export interface PhotoPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type: "photo" | "gif" | "video" | "audio";
  width: number;
  height: number;
  duration?: number;
  latitude?: number;
  longitude?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64
}

/** Encrypted thumbnail payload. */
export interface ThumbnailPayload {
  v: number;
  photo_blob_id: string;
  width: number;
  height: number;
  /** Optional MIME type of the thumbnail data (defaults to "image/jpeg" for backwards compat) */
  mime_type?: string;
  data: string; // base64 JPEG or GIF
}

// ── Viewer / preload types ───────────────────────────────────────────────────

/** Crop/edit metadata stored per-photo. */
export interface CropMetadata {
  x: number;
  y: number;
  width: number;
  height: number;
  rotate: number;
  brightness?: number;
  trimStart?: number;
  trimEnd?: number;
}

/** A preloaded media entry cached in memory for instant swiping. */
export interface PreloadEntry {
  url: string;
  filename: string;
  mimeType: string;
  mediaType: MediaType;
  cropData: CropMetadata | null;
  isFavorite: boolean;
}

/** Photo metadata displayed in the viewer info panel. */
export interface PhotoInfoData {
  filename: string;
  mimeType: string;
  width?: number;
  height?: number;
  takenAt?: string | null;
  sizeBytes?: number;
  latitude?: number | null;
  longitude?: number | null;
  createdAt?: string;
  durationSecs?: number | null;
  cameraModel?: string | null;
  albumNames?: string[];
}
