import { useState, useEffect } from "react";
import { useLiveQuery } from "dexie-react-hooks";
import { db, type CachedPhoto } from "../db";
import { useAuthStore } from "../store/auth";

interface GalleryItem {
  id: string;
  blob_id: string;
  added_at: string;
}

// ── Item Tile (shows decrypted thumbnail if available) ────────────────────────

export function ItemTile({ item, onClick }: { item: GalleryItem; onClick: () => void }) {
  const cachedPhoto = useLiveQuery(
    () => db.photos.get(item.blob_id),
    [item.blob_id]
  );

  useEffect(() => {
    console.log(
      `[DIAG:SECURE_TILE] item.blob_id=${item.blob_id}, ` +
      `hasCachedPhoto=${!!cachedPhoto}, ` +
      `hasThumbnailData=${!!cachedPhoto?.thumbnailData}, ` +
      `serverSide=${cachedPhoto?.serverSide}, ` +
      `serverPhotoId=${cachedPhoto?.serverPhotoId}`
    );
  }, [item.blob_id, cachedPhoto]);

  if (cachedPhoto?.thumbnailData) {
    return (
      <div
        className="aspect-square bg-gray-200 dark:bg-gray-700 rounded-lg overflow-hidden cursor-pointer hover:opacity-90 transition-opacity"
        onClick={onClick}
      >
        <PhotoThumbnail photo={cachedPhoto} />
      </div>
    );
  }

  // Fallback for server-side photos: load thumbnail from server API
  if (cachedPhoto?.serverSide && cachedPhoto?.serverPhotoId) {
    return (
      <div
        className="aspect-square bg-gray-200 dark:bg-gray-700 rounded-lg overflow-hidden cursor-pointer hover:opacity-90 transition-opacity"
        onClick={onClick}
      >
        <PhotoThumbnail photo={cachedPhoto} />
      </div>
    );
  }

  return (
    <div
      className="aspect-square bg-gray-200 dark:bg-gray-700 rounded-lg flex items-center justify-center overflow-hidden cursor-pointer hover:opacity-90 transition-opacity"
      onClick={onClick}
    >
      <div className="text-center text-gray-400">
        <span className="text-2xl block mb-1">🔐</span>
        <span className="text-xs">Encrypted</span>
      </div>
    </div>
  );
}

// ── Photo Thumbnail helper ────────────────────────────────────────────────────

export function PhotoThumbnail({ photo }: { photo: CachedPhoto }) {
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    if (photo.thumbnailData) {
      const mime = photo.thumbnailMimeType || (photo.mediaType === "gif" ? "image/gif" : "image/jpeg");
      const url = URL.createObjectURL(
        new Blob([photo.thumbnailData], { type: mime })
      );
      setSrc(url);
      return () => URL.revokeObjectURL(url);
    } else if (photo.serverSide && photo.serverPhotoId) {
      // Server-side (autoscanned) photos: load thumbnail from server API
      // (same fallback that MediaTile uses in the main gallery)
      const token = useAuthStore.getState().accessToken;
      setSrc(`/api/photos/${photo.serverPhotoId}/thumbnail?token=${token}`);
      console.log(
        `[DIAG:SECURE_PICKER] Server-side thumbnail fallback for ${photo.blobId}, serverPhotoId=${photo.serverPhotoId}`
      );
    } else {
      console.log(
        `[DIAG:SECURE_PICKER] No thumbnail data for ${photo.blobId}, serverSide=${photo.serverSide}, serverPhotoId=${photo.serverPhotoId}`
      );
    }
  }, [photo.thumbnailData, photo.thumbnailMimeType, photo.mediaType, photo.serverSide, photo.serverPhotoId, photo.blobId]);

  if (src) {
    return (
      <img
        src={src}
        alt={photo.filename}
        className="w-full h-full object-cover"
        loading="lazy"
      />
    );
  }

  return (
    <div className="w-full h-full flex items-center justify-center text-gray-400 text-xs px-1 text-center bg-gray-100 dark:bg-gray-700">
      {photo.filename}
    </div>
  );
}
