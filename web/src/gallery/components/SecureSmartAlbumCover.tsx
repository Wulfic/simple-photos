/**
 * SecureSmartAlbumCover — cover thumbnail for a built-in secure smart album.
 *
 * Unlike {@link SecureAlbumCover}, this takes the cover item *directly* (from
 * the already-fetched aggregate `/galleries/secure/items` feed) and issues NO
 * extra request — it only reads the local IDB cache for the item's decrypt
 * hints, then renders the decrypted thumbnail. Falls back to the lock glyph.
 */
import { useEffect, useState } from "react";
import { db } from "../../db";
import { useThumbnailLoader } from "../hooks/useThumbnailLoader";
import type { ThumbnailSource } from "../types";
import type { SecureGalleryItem } from "../../api/galleries";

export default function SecureSmartAlbumCover({
  item,
}: {
  item: SecureGalleryItem;
}) {
  const [source, setSource] = useState<ThumbnailSource | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const cached = await db.photos.get(item.blob_id);
        if (cancelled) return;
        setSource({
          blobId: item.blob_id,
          storageBlobId: cached?.storageBlobId,
          encryptedThumbBlobId: item.encrypted_thumb_blob_id ?? undefined,
          serverPhotoId: cached?.serverPhotoId,
          serverSide: cached?.serverSide,
          thumbnailMimeType: cached?.thumbnailMimeType,
        });
      } catch {
        /* leave the lock-glyph fallback in place */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [item.blob_id, item.encrypted_thumb_blob_id]);

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
