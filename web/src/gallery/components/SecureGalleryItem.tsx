/**
 * SecureGalleryItem — wrapper that bridges GalleryItem → ThumbnailTile.
 *
 * Performs the IDB lookup (via useSecureItemSource) and renders a
 * ThumbnailTile with the resolved thumbnail source.
 */
import { useSecureItemSource } from "../hooks/useSecureItemSource";
import ThumbnailTile from "./ThumbnailTile";

interface GalleryItem {
  id: string;
  blob_id: string;
  encrypted_thumb_blob_id?: string | null;
  width?: number | null;
  height?: number | null;
  media_type?: string | null;
  photo_subtype?: string | null;
  burst_id?: string | null;
  duration_secs?: number | null;
  /** Non-destructive crop/edit JSON stored on the secure item (#31). */
  crop_metadata?: string | null;
}

export default function SecureGalleryItem({
  item,
  burstCount,
  onClick,
}: {
  item: GalleryItem;
  burstCount?: number;
  onClick: () => void;
}) {
  const { source, mediaType, filename, photoSubtype, duration } = useSecureItemSource(item);

  return (
    <ThumbnailTile
      source={source}
      mediaType={mediaType}
      filename={filename}
      photoSubtype={photoSubtype}
      burstCount={burstCount}
      duration={duration}
      // Apply the secure item's own crop non-destructively at display time (#31),
      // exactly like a regular photo — ThumbnailTile handles the transform
      // (crop / rotate / brightness), including for GIFs. cropData is a JSON string.
      cropData={item.crop_metadata ?? undefined}
      width={item.width ?? undefined}
      height={item.height ?? undefined}
      onClick={onClick}
    />
  );
}
