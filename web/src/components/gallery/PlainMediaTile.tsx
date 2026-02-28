import { useState, useEffect, useRef } from "react";
import useLongPress from "../../hooks/useLongPress";
import { getCachedThumbnail, cacheThumbnail, formatDuration, type PlainPhoto } from "../../utils/gallery";
import { api } from "../../api/client";
import { useAuthStore } from "../../store/auth";

export interface PlainMediaTileProps {
  photo: PlainPhoto;
  onClick: () => void;
  onLongPress?: () => void;
  selectionMode?: boolean;
  isSelected?: boolean;
}

export default function PlainMediaTile({ photo, onClick, onLongPress, selectionMode, isSelected }: PlainMediaTileProps) {
  const [visible, setVisible] = useState(false);
  const [thumbSrc, setThumbSrc] = useState<string | null>(null);
  const tileRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = tileRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "200px" }
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Fetch thumbnail with cache-first strategy
  useEffect(() => {
    if (!visible) return;
    let cancelled = false;
    (async () => {
      try {
        // Try cache first
        const cached = await getCachedThumbnail(photo.id);
        if (cached && !cancelled) {
          setThumbSrc(cached);
          return;
        }

        // Fetch from server
        const { accessToken } = useAuthStore.getState();
        const headers: Record<string, string> = { "X-Requested-With": "SimplePhotos" };
        if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
        const res = await fetch(api.photos.thumbUrl(photo.id), { headers });
        if (!res.ok || cancelled) return;
        const blob = await res.blob();
        if (cancelled) return;

        // Cache and display
        const url = await cacheThumbnail(photo.id, blob);
        if (!cancelled) setThumbSrc(url);
      } catch {
        // Thumbnail load failed — show filename instead
      }
    })();
    return () => { cancelled = true; };
  }, [visible, photo.id]);

  const longPress = useLongPress(() => onLongPress?.(), 500);

  return (
    <div
      ref={tileRef}
      className={`relative aspect-square bg-gray-100 dark:bg-gray-700 rounded overflow-hidden cursor-pointer hover:opacity-90 transition-opacity group ${isSelected ? "ring-2 ring-blue-500" : ""}`}
      onClick={(e) => { if (longPress.wasLongPress()) { e.preventDefault(); return; } onClick(); }}
      onTouchStart={longPress.onTouchStart}
      onTouchEnd={longPress.onTouchEnd}
      onTouchMove={longPress.onTouchMove}
      onContextMenu={(e) => e.preventDefault()}
    >
      {thumbSrc ? (
        <img
          src={thumbSrc}
          alt={photo.filename}
          className="w-full h-full object-cover"
          loading="lazy"
        />
      ) : (
        <div className="w-full h-full flex items-center justify-center text-gray-400 text-xs px-1 text-center">
          {photo.filename}
        </div>
      )}

      {/* Favorite badge */}
      {photo.is_favorite && (
        <div className="absolute top-1 right-1 text-yellow-400 text-sm drop-shadow-lg">
          ★
        </div>
      )}

      {/* Media type badge */}
      {photo.media_type === "video" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>▶</span>
          {photo.duration_secs ? <span>{formatDuration(photo.duration_secs)}</span> : null}
        </div>
      )}
      {photo.media_type === "gif" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded">
          GIF
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
