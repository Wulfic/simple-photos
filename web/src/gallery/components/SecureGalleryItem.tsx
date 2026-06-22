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
      width={item.width ?? undefined}
      height={item.height ?? undefined}
      onClick={onClick}
    />
  );
}
