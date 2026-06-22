/**
 * SecureAlbumCover — decrypted cover thumbnail for a secure album card.
 *
 * Mirrors the Android GalleryCoverThumbnail: lazily fetches the album's items,
 * builds a ThumbnailSource from the newest item, and renders its decrypted
 * thumbnail. Falls back to the lock glyph for empty albums / load failures.
 *
 * (Previously the web album list only ever showed a 🔒 emoji — covers worked on
 * Android but not on web.)
 */
import { useEffect, useState } from "react";
import { api } from "../../api/client";
import { db } from "../../db";
import { useThumbnailLoader } from "../hooks/useThumbnailLoader";
import type { ThumbnailSource } from "../types";

export default function SecureAlbumCover({
  galleryId,
  galleryToken,
  itemCount,
}: {
  galleryId: string;
  galleryToken: string;
  itemCount: number;
}) {
  const [source, setSource] = useState<ThumbnailSource | null>(null);

  useEffect(() => {
    let cancelled = false;
    if (itemCount === 0) {
      setSource(null);
      return;
    }
    (async () => {
      try {
        const res = await api.secureGalleries.listItems(galleryId, galleryToken);
        const first = res.items[0];
        if (!first || cancelled) return;
        const cached = await db.photos.get(first.blob_id);
        if (cancelled) return;
        setSource({
          blobId: first.blob_id,
          storageBlobId: cached?.storageBlobId,
          encryptedThumbBlobId: first.encrypted_thumb_blob_id ?? undefined,
          serverPhotoId: cached?.serverPhotoId,
          serverSide: cached?.serverSide,
          thumbnailData: cached?.thumbnailData,
          thumbnailMimeType: cached?.thumbnailMimeType,
        });
      } catch {
        /* leave the lock-glyph fallback in place */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [galleryId, galleryToken, itemCount]);

  if (!source) return <span className="text-xl">🔒</span>;
  return <CoverImg source={source} />;
}

function CoverImg({ source }: { source: ThumbnailSource }) {
  const thumb = useThumbnailLoader(source, true);
  if (!thumb.url) return <span className="text-xl">🔒</span>;
  return (
    <img src={thumb.url} alt="Album cover" className="w-full h-full object-cover" />
  );
}
