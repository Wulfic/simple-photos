import { useState, useEffect } from "react";
import { useLiveQuery } from "dexie-react-hooks";
import { db, type CachedPhoto } from "../db";
import { useAuthStore } from "../store/auth";
import { blobsApi } from "../api/blobs";
import { decrypt } from "../crypto/crypto";

interface GalleryItem {
  id: string;
  blob_id: string;
  added_at: string;
  encrypted_thumb_blob_id?: string | null;
}

// ── Item Tile (shows decrypted thumbnail if available) ────────────────────────

export function ItemTile({ item, onClick }: { item: GalleryItem; onClick: () => void }) {
  const cachedPhoto = useLiveQuery(
    () => db.photos.get(item.blob_id),
    [item.blob_id]
  );
  const [encThumbSrc, setEncThumbSrc] = useState<string | null>(null);

  useEffect(() => {
    console.log(
      `[DIAG:SECURE_TILE] item.blob_id=${item.blob_id}, ` +
      `hasCachedPhoto=${!!cachedPhoto}, ` +
      `hasThumbnailData=${!!cachedPhoto?.thumbnailData}, ` +
      `serverSide=${cachedPhoto?.serverSide}, ` +
      `serverPhotoId=${cachedPhoto?.serverPhotoId}, ` +
      `encThumbBlobId=${item.encrypted_thumb_blob_id}`
    );
  }, [item.blob_id, cachedPhoto, item.encrypted_thumb_blob_id]);

  // Download and decrypt the encrypted thumbnail blob when no local cache exists
  useEffect(() => {
    if (cachedPhoto?.thumbnailData) return; // already have local thumbnail
    if (cachedPhoto?.serverSide && cachedPhoto?.serverPhotoId) return; // server path available
    if (!item.encrypted_thumb_blob_id) return;

    let cancelled = false;
    (async () => {
      try {
        const encData = await blobsApi.download(item.encrypted_thumb_blob_id!);
        if (cancelled) return;
        const plaintext = await decrypt(encData);
        if (cancelled) return;
        const json = JSON.parse(new TextDecoder().decode(plaintext));
        const b64 = json.data as string;
        if (!b64) return;
        const binary = atob(b64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
        const url = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
        if (!cancelled) setEncThumbSrc(url);
      } catch (err) {
        console.warn(`[SECURE_TILE] Failed to decrypt thumbnail ${item.encrypted_thumb_blob_id}:`, err);
      }
    })();
    return () => { cancelled = true; };
  }, [cachedPhoto?.thumbnailData, cachedPhoto?.serverSide, cachedPhoto?.serverPhotoId, item.encrypted_thumb_blob_id]);

  // Clean up object URL on unmount
  useEffect(() => {
    return () => {
      if (encThumbSrc) URL.revokeObjectURL(encThumbSrc);
    };
  }, [encThumbSrc]);

  if (cachedPhoto?.thumbnailData) {
    return (
      <div
        className="w-full h-full bg-gray-200 dark:bg-gray-700 rounded-lg overflow-hidden cursor-pointer hover:opacity-90 transition-opacity"
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
        className="w-full h-full bg-gray-200 dark:bg-gray-700 rounded-lg overflow-hidden cursor-pointer hover:opacity-90 transition-opacity"
        onClick={onClick}
      >
        <PhotoThumbnail photo={cachedPhoto} />
      </div>
    );
  }

  // Decrypted encrypted thumbnail blob
  if (encThumbSrc) {
    return (
      <div
        className="w-full h-full bg-gray-200 dark:bg-gray-700 rounded-lg overflow-hidden cursor-pointer hover:opacity-90 transition-opacity"
        onClick={onClick}
      >
        <img
          src={encThumbSrc}
          alt="Encrypted thumbnail"
          className="w-full h-full object-cover"
          loading="lazy"
        />
      </div>
    );
  }

  return (
    <div
      className="w-full h-full bg-gray-200 dark:bg-gray-700 rounded-lg flex items-center justify-center overflow-hidden cursor-pointer hover:opacity-90 transition-opacity"
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
