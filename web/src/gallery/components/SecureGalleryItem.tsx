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
}

export default function SecureGalleryItem({
  item,
  onClick,
}: {
  item: GalleryItem;
  onClick: () => void;
}) {
  const { source, mediaType, filename } = useSecureItemSource(item);

  return (
    <ThumbnailTile
      source={source}
      mediaType={mediaType}
      filename={filename}
      onClick={onClick}
    />
  );
}
