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
  photoSubtype,
  burstCount,
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

      {/* Photo subtype badges */}
      {photoSubtype === "burst" && (
        <div className="absolute top-1 left-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M4 6h16M4 10h16M4 14h16" />
          </svg>
          {burstCount && burstCount > 1 ? <span>{burstCount}</span> : null}
        </div>
      )}
      {photoSubtype === "motion" && (
        <div className="absolute top-1 left-1 bg-black/60 text-white text-[10px] font-bold px-1.5 py-0.5 rounded">
          LIVE
        </div>
      )}
      {(photoSubtype === "panorama" || photoSubtype === "equirectangular") && (
        <div className="absolute top-1 left-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6.115 5.19l.319 1.913A6 6 0 008.11 10.36L9.75 12l-.387.775c-.217.433-.132.956.21 1.298l1.348 1.348c.21.21.329.497.329.795v1.089c0 .426.24.815.622 1.006l.153.076c.433.217.956.132 1.298-.21l.723-.723a8.7 8.7 0 002.288-4.042 1.087 1.087 0 00-.358-1.099l-1.33-1.108c-.251-.21-.582-.299-.905-.245l-1.17.195a1.125 1.125 0 01-.98-.314l-.295-.295a1.125 1.125 0 010-1.591l.13-.132a1.125 1.125 0 011.3-.21l.603.302a.809.809 0 001.086-1.086L14.25 7.5l1.256-.837a4.5 4.5 0 001.528-1.732l.146-.292M6.115 5.19A9 9 0 1017.18 4.64M6.115 5.19A8.965 8.965 0 0112 3c1.929 0 3.716.607 5.18 1.64" />
          </svg>
          {photoSubtype === "equirectangular" ? "360°" : "PANO"}
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
