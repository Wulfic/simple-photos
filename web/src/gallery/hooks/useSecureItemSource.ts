/**
 * Hook to build a ThumbnailSource from a secure gallery item.
 *
 * Bridges the gap between the server's GalleryItem shape and the
 * ThumbnailSource expected by ThumbnailTile. Performs an IDB lookup
 * to get cached thumbnail data when available.
 */
import { useLiveQuery } from "dexie-react-hooks";
import { db } from "../../db";
import type { ThumbnailSource } from "../types";

interface GalleryItem {
  id: string;
  blob_id: string;
  encrypted_thumb_blob_id?: string | null;
  media_type?: string | null;
  photo_subtype?: string | null;
  duration_secs?: number | null;
}

/**
 * Given a secure gallery item, return a ThumbnailSource that
 * useThumbnailLoader can consume.  Automatically looks up the
 * CachedPhoto in IDB for local thumbnail data.
 */
export function useSecureItemSource(item: GalleryItem): {
  source: ThumbnailSource;
  mediaType: "photo" | "gif" | "video" | "audio";
  filename: string;
  photoSubtype: string | undefined;
  duration: number | undefined;
} {
  const cachedPhoto = useLiveQuery(
    () => db.photos.get(item.blob_id),
    [item.blob_id],
  );

  const source: ThumbnailSource = {
    blobId: item.blob_id,
    storageBlobId: cachedPhoto?.storageBlobId,
    encryptedThumbBlobId: item.encrypted_thumb_blob_id ?? undefined,
    serverPhotoId: cachedPhoto?.serverPhotoId,
    serverSide: cachedPhoto?.serverSide,
    thumbnailData: cachedPhoto?.thumbnailData,
    thumbnailMimeType: cachedPhoto?.thumbnailMimeType,
  };

  const mediaType = (
    item.media_type as "photo" | "gif" | "video" | "audio" | null
  ) ?? cachedPhoto?.mediaType ?? "photo";

  const filename = cachedPhoto?.filename ?? item.blob_id;

  // Subtype + duration come from the server item (preferred) and fall back to
  // the cached photo, so the secure tiles show PANO/360/LIVE badges and video
  // durations just like the main gallery.
  const photoSubtype = item.photo_subtype ?? cachedPhoto?.photoSubtype ?? undefined;
  const duration = item.duration_secs ?? cachedPhoto?.duration ?? undefined;

  return { source, mediaType, filename, photoSubtype, duration };
}
