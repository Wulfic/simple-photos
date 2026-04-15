/** Gallery thumbnail tile for encrypted-mode photos. Creates object URLs
 *  from decrypted IndexedDB thumbnail data, lazy-loaded via IntersectionObserver.
 *  GIF tiles whose thumbnail is already animated (mime=image/gif) display the
 *  thumbnail directly; large GIFs with a static JPEG thumbnail load the full
 *  encrypted blob when scrolled into view. */
import { useState, useEffect, useRef } from "react";
import { db, type CachedPhoto } from "../../db";
import { api } from "../../api/client";
import useLongPress from "../../hooks/useLongPress";
import { thumbnailSrc, formatDuration } from "../../utils/gallery";
import { loadFullGif } from "../../utils/gifLoader";

import { getThumbnailStyle } from "../../utils/thumbnailCss";

export interface MediaTileProps {
  photo: CachedPhoto;
  onClick: () => void;
  onLongPress?: () => void;
  selectionMode?: boolean;
  isSelected?: boolean;
}

export default function MediaTile({ photo, onClick, onLongPress, selectionMode, isSelected }: MediaTileProps) {
  const isGif = photo.mediaType === "gif";
  // Only consider a thumbnail animated when the MIME type is explicitly "image/gif"
  // (not the fallback). Old GIFs may have JPEG thumbnails with no stored MIME type.
  const hasAnimatedThumb = isGif && photo.thumbnailMimeType === "image/gif";
  // GIFs without an explicitly animated thumbnail need the full blob for animation.
  const needsFullLoad = isGif && !hasAnimatedThumb;

  const [src, setSrc] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const [inView, setInView] = useState(false);
  const tileRef = useRef<HTMLDivElement>(null);
  // Stable blob URL for the thumbnail — keyed by blobId to avoid recreating on
  // every Dexie re-query (which returns new ArrayBuffer references each time).
  // Recreating would set a new img src, restarting the GIF animation from frame 1.
  const thumbUrlRef = useRef<string | null>(null);
  const thumbCreatedForRef = useRef<string | undefined>(undefined);
  const fullGifUrl = useRef<string | null>(null);

  // Viewport tracking: persistent for GIFs that need full-blob loading (in/out swap),
  // one-shot for everything else.
  useEffect(() => {
    const el = tileRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) setVisible(true);
        if (needsFullLoad) setInView(entry.isIntersecting);
        else if (entry.isIntersecting) observer.disconnect();
      },
      { rootMargin: "200px" }
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [needsFullLoad]);

  // Create and cache the thumbnail blob URL.
  // Keyed by photo.blobId so it is only created ONCE per photo — subsequent
  // Dexie re-renders that return a new ArrayBuffer reference with identical bytes
  // do NOT recreate the URL, preventing the GIF from restarting on every sync cycle.
  useEffect(() => {
    if (!photo.thumbnailData) return;
    if (thumbUrlRef.current && thumbCreatedForRef.current === photo.blobId) {
      // Same photo, URL already valid — show it if we've become visible
      if (visible && !fullGifUrl.current) setSrc(thumbUrlRef.current);
      return;
    }
    // New photo (or first load) — revoke old URL and create fresh one
    if (thumbUrlRef.current) URL.revokeObjectURL(thumbUrlRef.current);
    const mime = photo.thumbnailMimeType || "image/jpeg";
    thumbUrlRef.current = thumbnailSrc(photo.thumbnailData, mime);
    thumbCreatedForRef.current = photo.blobId;
    if (visible && !fullGifUrl.current) setSrc(thumbUrlRef.current);
  }, [visible, photo.thumbnailData, photo.blobId, photo.thumbnailMimeType]);

  // Large GIFs: fetch full animated blob when in view.
  useEffect(() => {
    if (!needsFullLoad || !inView || fullGifUrl.current) return;
    let cancelled = false;
    const id = photo.storageBlobId ?? photo.blobId;
    loadFullGif(id, photo.serverSide ? photo.serverPhotoId : undefined).then((url) => {
      if (!cancelled && url) {
        fullGifUrl.current = url;
        setSrc(url);
      }
    });
    return () => { cancelled = true; };
  }, [needsFullLoad, inView, photo.storageBlobId, photo.blobId, photo.serverSide, photo.serverPhotoId]);

  // Large GIFs: swap between full animated file (in view) and thumbnail (out of view).
  useEffect(() => {
    if (!needsFullLoad || !fullGifUrl.current) return;
    if (inView) {
      setSrc(fullGifUrl.current);
    } else if (thumbUrlRef.current) {
      setSrc(thumbUrlRef.current);
    }
  }, [needsFullLoad, inView]);

  // Cleanup on unmount
  useEffect(() => () => {
    if (thumbUrlRef.current) URL.revokeObjectURL(thumbUrlRef.current);
    if (fullGifUrl.current) URL.revokeObjectURL(fullGifUrl.current);
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
          {/* GIFs use object-cover (fills tile, no letterbox bars) without crop transforms.
              JustifiedGrid already sizes the tile to match the GIF's aspect ratio so
              object-cover doesn't crop. Never apply getThumbnailStyle to GIFs — the crop
              scale/translate transform would zoom in and clip the animation. */}
          <img
            src={src}
            alt={photo.filename}
            className="w-full h-full object-cover"
            loading="lazy"
            style={isGif ? undefined : getThumbnailStyle(photo.cropData)}
            onLoad={(e) => {
              const img = e.currentTarget;
              const nw = img.naturalWidth;
              const nh = img.naturalHeight;
              // Self-heal: if the thumbnail's orientation disagrees with stored
              // photo dimensions, swap them in IDB so the grid AR is correct.
              // Also push the fix to the server so the sync engine stops
              // overwriting with wrong values.
              if (
                nw > 0 && nh > 0 && nw !== nh &&
                photo.width > 0 && photo.height > 0 &&
                !photo.cropData &&
                (nw > nh) !== (photo.width > photo.height)
              ) {
                const correctedW = photo.height;
                const correctedH = photo.width;
                db.photos.update(photo.blobId, {
                  width: correctedW,
                  height: correctedH,
                });
                // Push corrected dimensions to the server
                if (photo.serverPhotoId) {
                  api.photos.batchUpdateDimensions([{
                    photo_id: photo.serverPhotoId,
                    width: correctedW,
                    height: correctedH,
                  }]).catch(() => { /* non-fatal */ });
                }
              }
            }}
          />
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
