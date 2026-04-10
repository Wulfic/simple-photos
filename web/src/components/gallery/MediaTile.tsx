/** Gallery thumbnail tile for encrypted-mode photos. Creates object URLs
 *  from decrypted IndexedDB thumbnail data, lazy-loaded via IntersectionObserver.
 *  GIF thumbnails animate only while the tile is in the viewport. */
import { useState, useEffect, useRef } from "react";
import type { CachedPhoto } from "../../db";
import useLongPress from "../../hooks/useLongPress";
import { thumbnailSrc, formatDuration, extractStaticFrame } from "../../utils/gallery";

import { getThumbnailStyle } from "../../utils/thumbnailCss";

/** Resolve the correct MIME type for a photo's thumbnail data.
 *  Prefers the explicit thumbnailMimeType field, falls back to
 *  "image/gif" for GIF media type, or "image/jpeg" otherwise. */
function thumbMime(photo: CachedPhoto): string {
  if (photo.thumbnailMimeType) return photo.thumbnailMimeType;
  if (photo.mediaType === "gif") return "image/gif";
  return "image/jpeg";
}

export interface MediaTileProps {
  photo: CachedPhoto;
  onClick: () => void;
  onLongPress?: () => void;
  selectionMode?: boolean;
  isSelected?: boolean;
}

export default function MediaTile({ photo, onClick, onLongPress, selectionMode, isSelected }: MediaTileProps) {
  const isAnimatedGif = photo.mediaType === "gif" && thumbMime(photo) === "image/gif";
  const [src, setSrc] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const [inView, setInView] = useState(false);
  const tileRef = useRef<HTMLDivElement>(null);
  const gifAnimatedUrl = useRef<string | null>(null);
  const gifStaticUrl = useRef<string | null>(null);
  const inViewRef = useRef(false);

  // Viewport tracking: one-shot for non-GIF, persistent for animated GIFs
  useEffect(() => {
    const el = tileRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (isAnimatedGif) {
          if (entry.isIntersecting) setVisible(true);
          setInView(entry.isIntersecting);
          inViewRef.current = entry.isIntersecting;
        } else if (entry.isIntersecting) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "200px" }
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [isAnimatedGif]);

  // Non-animated: create object URL when first visible
  useEffect(() => {
    if (isAnimatedGif || !visible) return;
    if (photo.thumbnailData) {
      const url = thumbnailSrc(photo.thumbnailData, thumbMime(photo));
      setSrc(url);
      return () => URL.revokeObjectURL(url);
    }
  }, [isAnimatedGif, visible, photo.thumbnailData, photo.thumbnailMimeType, photo.mediaType]);

  // Animated GIF: create animated + static URLs once, swap based on viewport
  useEffect(() => {
    if (!isAnimatedGif || !visible || !photo.thumbnailData) return;
    if (!gifAnimatedUrl.current) {
      gifAnimatedUrl.current = thumbnailSrc(photo.thumbnailData, "image/gif");
      extractStaticFrame(gifAnimatedUrl.current)
        .then((url) => {
          gifStaticUrl.current = url;
          if (!inViewRef.current) setSrc(url);
        })
        .catch(() => { /* fall back to always animated */ });
    }
    setSrc(inView ? gifAnimatedUrl.current : (gifStaticUrl.current ?? gifAnimatedUrl.current));
  }, [isAnimatedGif, visible, inView, photo.thumbnailData]);

  // Cleanup GIF URLs on unmount
  useEffect(() => () => {
    if (gifAnimatedUrl.current) URL.revokeObjectURL(gifAnimatedUrl.current);
    if (gifStaticUrl.current) URL.revokeObjectURL(gifStaticUrl.current);
  }, []);

  const longPress = useLongPress(() => onLongPress?.(), 500);

  return (
    <div
      ref={tileRef}
      className={`relative w-full h-full bg-gray-100 dark:bg-gray-700 overflow-hidden cursor-pointer hover:opacity-90 transition-opacity group ${isSelected ? "ring-2 ring-blue-500" : ""}`}
      onClick={(e) => { if (longPress.wasLongPress()) { e.preventDefault(); return; } onClick(); }}
      onTouchStart={longPress.onTouchStart}
      onTouchEnd={longPress.onTouchEnd}
      onTouchMove={longPress.onTouchMove}
      onContextMenu={(e) => e.preventDefault()}
    >
      {src ? (
        <>
          <img src={src} alt={photo.filename} className="w-full h-full object-cover" loading="lazy" style={getThumbnailStyle(photo.cropData)} />
          {/* Filename overlay — only for audio files */}
          {photo.mediaType === "audio" && (
            <div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/60 to-transparent px-1 pb-0.5 pt-3 opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
              <span className="text-white text-[10px] leading-tight line-clamp-1 break-all drop-shadow">{photo.filename}</span>
            </div>
          )}
        </>
      ) : (
        <div className="w-full h-full flex flex-col items-center justify-center gap-1.5 px-1 text-center">
          {!photo.thumbnailData ? (
            <>
              <div className="w-5 h-5 border-2 border-gray-300 dark:border-gray-500 border-t-blue-500 dark:border-t-blue-400 rounded-full animate-spin" />
              <span className="text-[10px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide">Queued</span>
            </>
          ) : (
            <span className="text-gray-400 text-xs">{photo.filename}</span>
          )}
        </div>
      )}

      {/* Media type badge */}
      {photo.mediaType === "video" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>▶</span>
          {photo.duration ? <span>{formatDuration(photo.duration)}</span> : null}
        </div>
      )}
      {photo.mediaType === "gif" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded">
          GIF
        </div>
      )}
      {photo.mediaType === "audio" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>♫</span>
          {photo.duration ? <span>{formatDuration(photo.duration)}</span> : null}
        </div>
      )}

      {/* Selection indicator */}
      {selectionMode && (
        <div className={`absolute top-1.5 right-1.5 w-5 h-5 rounded-full border-2 flex items-center justify-center transition-colors ${
          isSelected ? "bg-green-500 border-green-500" : "bg-white/80 border-gray-400"
        }`}>
          {isSelected && (
            <svg className="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}><path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" /></svg>
          )}
        </div>
      )}
    </div>
  );
}
