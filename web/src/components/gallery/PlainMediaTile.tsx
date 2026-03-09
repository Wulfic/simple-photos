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
  const [isQueued, setIsQueued] = useState(false);
  const tileRef = useRef<HTMLDivElement>(null);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

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

  // Fetch thumbnail with cache-first strategy, retries on 202 (pending)
  useEffect(() => {
    if (!visible) return;
    let cancelled = false;

    async function fetchThumb() {
      try {
        // Try cache first
        const cached = await getCachedThumbnail(photo.id);
        if (cached && !cancelled) {
          setThumbSrc(cached);
          setIsQueued(false);
          return;
        }

        // Fetch from server
        const { accessToken } = useAuthStore.getState();
        const headers: Record<string, string> = { "X-Requested-With": "SimplePhotos" };
        if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
        const res = await fetch(api.photos.thumbUrl(photo.id), { headers });

        if (cancelled) return;

        // 202 = thumbnail pending (conversion/generation in progress)
        if (res.status === 202) {
          setIsQueued(true);
          // Retry after 10 seconds
          retryTimerRef.current = setTimeout(() => {
            if (!cancelled) fetchThumb();
          }, 10_000);
          return;
        }

        if (!res.ok) return;
        const blob = await res.blob();
        if (cancelled) return;

        // Cache and display
        const url = await cacheThumbnail(photo.id, blob);
        if (!cancelled) {
          setThumbSrc(url);
          setIsQueued(false);
        }
      } catch {
        // Thumbnail load failed — treat as queued if we don't have one yet
        if (!cancelled && !thumbSrc) {
          setIsQueued(true);
          retryTimerRef.current = setTimeout(() => {
            if (!cancelled) fetchThumb();
          }, 10_000);
        }
      }
    }

    fetchThumb();
    return () => {
      cancelled = true;
      if (retryTimerRef.current) clearTimeout(retryTimerRef.current);
    };
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
        <>
          <img
            src={thumbSrc}
            alt={photo.filename}
            className="w-full h-full object-cover"
            loading="lazy"
          />
          <div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/60 to-transparent px-1 pb-0.5 pt-3 opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
            <span className="text-white text-[10px] leading-tight line-clamp-1 break-all drop-shadow">{photo.filename}</span>
          </div>
        </>
      ) : (
        <div className="w-full h-full flex flex-col items-center justify-center gap-1.5 px-1 text-center">
          {isQueued ? (
            <>
              <div className="w-5 h-5 border-2 border-gray-300 dark:border-gray-500 border-t-blue-500 dark:border-t-blue-400 rounded-full animate-spin" />
              <span className="text-[10px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide">Queued</span>
            </>
          ) : (
            <span className="text-gray-400 text-xs">{photo.filename}</span>
          )}
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
      {photo.media_type === "audio" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>♫</span>
          {photo.duration_secs ? <span>{formatDuration(photo.duration_secs)}</span> : null}
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
