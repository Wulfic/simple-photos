/**
 * PickerThumbnail — lightweight thumbnail for photo picker grids.
 *
 * Used in the secure gallery "Add Photos" picker where we just need
 * a simple thumbnail image without badges, selection indicators,
 * GIF autoplay, or crop transforms.
 */
import { useThumbnailLoader } from "../hooks/useThumbnailLoader";
import type { ThumbnailSource } from "../types";

export default function PickerThumbnail({ source, filename }: {
  source: ThumbnailSource;
  filename: string;
}) {
  const { url, state } = useThumbnailLoader(source, true);

  if (url) {
    return (
      <img
        src={url}
        alt={filename}
        className="w-full h-full object-cover"
        loading="lazy"
      />
    );
  }

  return (
    <div className="w-full h-full flex items-center justify-center text-gray-400 text-xs px-1 text-center bg-gray-100 dark:bg-gray-700">
      {state === "loading" ? (
        <div className="w-4 h-4 border-2 border-gray-300 border-t-blue-500 rounded-full animate-spin" />
      ) : (
        filename
      )}
    </div>
  );
}
