/**
 * ThumbnailTile — unified gallery tile component.
 *
 * Replaces MediaTile (main gallery), SecureGalleryTile.ItemTile (secure gallery),
 * and PhotoThumbnail (secure gallery helper) with a single component that handles
 * all view contexts: main gallery, secure gallery, albums, shared albums.
 *
 * Uses the `useThumbnailLoader` hook for decrypted/server thumbnail resolution
 * and `useGifAutoplay` for large GIF full-blob loading.
 */
import { useRef, useEffect, useState } from "react";
import useLongPress from "../../hooks/useLongPress";
import { useThumbnailLoader } from "../hooks/useThumbnailLoader";
import { useGifAutoplay } from "../hooks/useGifAutoplay";
import { getThumbnailStyle } from "../../utils/thumbnailCss";
import { formatDuration } from "../../utils/gallery";
import type { ThumbnailTileProps } from "../types";

export default function ThumbnailTile({
  source,
  mediaType,
  filename,
  cropData,
  duration,
  onClick,
  onLongPress,
  selectionMode,
  isSelected,
  onDimensionMismatch,
}: ThumbnailTileProps) {
  const tileRef = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);

  const isGif = mediaType === "gif";
  const hasAnimatedThumb = isGif && source.thumbnailMimeType === "image/gif";
  const needsFullGifLoad = isGif && !hasAnimatedThumb;

  // Lazy visibility gate — only start loading when the tile enters the viewport
  useEffect(() => {
    const el = tileRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          if (!needsFullGifLoad) observer.disconnect(); // One-shot for non-GIF
        }
      },
      { rootMargin: `${Math.max(200, Math.round(window.innerHeight * 0.5))}px` },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [needsFullGifLoad]);

  // Thumbnail loading (cache → IDB → encrypted blob → server fallback)
  const thumb = useThumbnailLoader(source, visible);

  // GIF autoplay (large GIFs only — small GIFs play directly from thumbnail)
  const gif = useGifAutoplay(
    tileRef,
    source.storageBlobId ?? source.blobId,
    source.serverSide ? source.serverPhotoId : undefined,
    needsFullGifLoad && visible,
  );

  // The displayed URL: full GIF blob when in view, otherwise the thumbnail
  const displayUrl = gif.fullGifUrl ?? thumb.url;

  // Long press for selection mode
  const longPress = useLongPress(() => onLongPress?.(), 500);

  return (
    <div
      ref={tileRef}
      className={`relative w-full h-full bg-gray-100 dark:bg-gray-700 overflow-hidden cursor-pointer hover:opacity-90 transition-opacity group ${isSelected ? "ring-2 ring-blue-500" : ""}`}
      onClick={(e) => {
        if (longPress.wasLongPress()) { e.preventDefault(); return; }
        onClick();
      }}
      onTouchStart={longPress.onTouchStart}
      onTouchEnd={longPress.onTouchEnd}
      onTouchMove={longPress.onTouchMove}
      onContextMenu={(e) => e.preventDefault()}
    >
      {displayUrl ? (
        <>
          {/* GIFs use object-cover without crop transforms (breaks animation).
              JustifiedGrid already sizes the tile to match the GIF's aspect ratio. */}
          <img
            src={displayUrl}
            alt={filename}
            className="w-full h-full object-cover"
            loading="lazy"
            style={isGif ? undefined : getThumbnailStyle(cropData)}
            onLoad={onDimensionMismatch ? (e) => {
              const img = e.currentTarget;
              const nw = img.naturalWidth;
              const nh = img.naturalHeight;
              // Self-heal: if thumbnail orientation disagrees with stored dimensions
              if (nw > 0 && nh > 0 && nw !== nh && !cropData) {
                onDimensionMismatch(nw, nh);
              }
            } : undefined}
          />

          {/* Filename overlay for audio files */}
          {mediaType === "audio" && (
            <div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/60 to-transparent px-1 pb-0.5 pt-3 opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
              <span className="text-white text-[10px] leading-tight line-clamp-1 break-all drop-shadow">{filename}</span>
            </div>
          )}
        </>
      ) : (
        <div className="w-full h-full flex flex-col items-center justify-center gap-1.5 px-1 text-center">
          {thumb.state === "loading" || thumb.state === "placeholder" ? (
            !source.thumbnailData ? (
              <>
                <div className="w-5 h-5 border-2 border-gray-300 dark:border-gray-500 border-t-blue-500 dark:border-t-blue-400 rounded-full animate-spin" />
                <span className="text-[10px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide">Queued</span>
              </>
            ) : (
              <span className="text-gray-400 text-xs">{filename}</span>
            )
          ) : thumb.state === "error" ? (
            <div className="text-center text-gray-400">
              <span className="text-2xl block mb-1">🔐</span>
              <span className="text-xs">Encrypted</span>
            </div>
          ) : (
            <span className="text-gray-400 text-xs">{filename}</span>
          )}
        </div>
      )}

      {/* Media type badges */}
      {mediaType === "video" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>▶</span>
          {duration ? <span>{formatDuration(duration)}</span> : null}
        </div>
      )}
      {mediaType === "gif" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded">
          GIF
        </div>
      )}
      {mediaType === "audio" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>♫</span>
          {duration ? <span>{formatDuration(duration)}</span> : null}
        </div>
      )}

      {/* Selection indicator */}
      {selectionMode && (
        <div className={`absolute top-1.5 right-1.5 w-5 h-5 rounded-full border-2 flex items-center justify-center transition-colors ${
          isSelected ? "bg-green-500 border-green-500" : "bg-white/80 border-gray-400"
        }`}>
          {isSelected && (
            <svg className="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
            </svg>
          )}
        </div>
      )}
    </div>
  );
}
