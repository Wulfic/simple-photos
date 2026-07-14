import { useEffect, useState, useRef } from "react";
import type { CachedPhoto } from "../db";
import { formatDuration } from "../utils/gallery";

// ── Thumbnail helper ──────────────────────────────────────────────────────────

export function ThumbnailImg({ photo }: { photo: CachedPhoto }) {
  const containerRef = useRef<HTMLDivElement>(null);
  // Lazy visibility gate. Without this, a grid that renders the whole library
  // (the regular-album "Add photos" picker feeds ~7000 photos in) built an
  // object URL for every tile at mount, freezing the tab so the picker never
  // appeared (#27). Mirror ThumbnailTile: only build the blob URL once the tile
  // nears the viewport. Fall back to eager when IntersectionObserver is absent
  // (SSR / tests) so a tile never stays permanently blank.
  const [visible, setVisible] = useState(
    typeof IntersectionObserver === "undefined",
  );
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    if (visible) return;
    const el = containerRef.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: `${Math.max(200, Math.round(window.innerHeight * 0.5))}px` },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [visible]);

  useEffect(() => {
    if (!visible || !photo.thumbnailData) {
      setSrc(null);
      return;
    }
    // Encrypted thumbnail stored in IndexedDB
    const mime = photo.thumbnailMimeType || (photo.mediaType === "gif" ? "image/gif" : "image/jpeg");
    const url = URL.createObjectURL(
      new Blob([photo.thumbnailData], { type: mime })
    );
    setSrc(url);
    return () => URL.revokeObjectURL(url);
  }, [visible, photo.thumbnailData, photo.thumbnailMimeType, photo.mediaType]);

  return (
    <div ref={containerRef} className="w-full h-full">
      {src ? (
        <img
          src={src}
          alt={photo.filename}
          className="w-full h-full object-cover"
          loading="lazy"
        />
      ) : (
        <div className="w-full h-full flex items-center justify-center text-fg-muted text-xs px-1 text-center bg-surface-raised">
          {photo.filename}
        </div>
      )}
    </div>
  );
}

// ── Album Tile ────────────────────────────────────────────────────────────────

export interface AlbumTileProps {
  photo: CachedPhoto;
  isSelectionMode: boolean;
  isSelected: boolean;
  onClick: () => void;
  onLongPress: () => void;
  onRemove: () => void;
}

export default function AlbumTile({ photo, isSelectionMode, isSelected, onClick, onLongPress, onRemove }: AlbumTileProps) {
  const longPressRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const didLongPress = useRef(false);

  function handlePointerDown() {
    didLongPress.current = false;
    longPressRef.current = setTimeout(() => {
      didLongPress.current = true;
      onLongPress();
      longPressRef.current = null;
    }, 500);
  }

  function handlePointerUp() {
    if (longPressRef.current) {
      clearTimeout(longPressRef.current);
      longPressRef.current = null;
    }
    if (!didLongPress.current) {
      onClick();
    }
  }

  function handlePointerLeave() {
    if (longPressRef.current) {
      clearTimeout(longPressRef.current);
      longPressRef.current = null;
    }
  }

  return (
    <div
      className={`relative w-full h-full bg-surface-raised rounded overflow-hidden cursor-pointer group ${
        isSelected ? "ring-2 ring-accent-500" : ""
      }`}
      onPointerDown={handlePointerDown}
      onPointerUp={handlePointerUp}
      onPointerLeave={handlePointerLeave}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="w-full h-full">
        <ThumbnailImg photo={photo} />
      </div>

      {/* Selection circle — always visible (top-right). Tapping toggles selection
          and enters selection mode if not already active. */}
      <button
        type="button"
        aria-label={isSelected ? "Deselect" : "Select"}
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
          if (isSelectionMode) {
            onClick();
          } else {
            onLongPress();
          }
        }}
        onPointerDown={(e) => e.stopPropagation()}
        className={`absolute top-1.5 right-1.5 w-5 h-5 rounded-full border-2 flex items-center justify-center transition-all z-10 ${
          isSelected
            ? "bg-green-500 border-green-500 shadow"
            : isSelectionMode
              ? "bg-white/80 border-gray-400 hover:bg-white"
              : "bg-white/40 border-white/70 opacity-70 hover:opacity-100 hover:bg-white/80 shadow-sm"
        }`}
      >
        {isSelected && (
          <svg className="w-3 h-3 text-white" fill="currentColor" viewBox="0 0 20 20">
            <path fillRule="evenodd" d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z" clipRule="evenodd" />
          </svg>
        )}
      </button>

      {/* Media type badge */}
      {photo.mediaType === "video" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>▶</span>
          {photo.duration ? (
            <span>{formatDuration(photo.duration)}</span>
          ) : null}
        </div>
      )}
      {photo.mediaType === "gif" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded">
          GIF
        </div>
      )}

      {/* Photo subtype badges (top-left) — pano/360, motion (LIVE), and burst
          stacks (with frame count) so smart albums match the main gallery. */}
      {(() => {
        const sub = photo.photoSubtype;
        const burstCount = (photo as CachedPhoto & { _burstCount?: number })._burstCount;
        if (sub === "panorama" || sub === "equirectangular") {
          return (
            <div className="absolute top-1 left-1 bg-black/60 text-white text-[10px] font-bold px-1.5 py-0.5 rounded">
              {sub === "equirectangular" ? "360°" : "PANO"}
            </div>
          );
        }
        if (sub === "motion") {
          return (
            <div className="absolute top-1 left-1 bg-black/60 text-white text-[10px] font-bold px-1.5 py-0.5 rounded">
              LIVE
            </div>
          );
        }
        if (photo.burstId) {
          return (
            <div className="absolute top-1 left-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
              <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 6h16M4 10h16M4 14h16" />
              </svg>
              {burstCount && burstCount > 1 ? <span>{burstCount}</span> : null}
            </div>
          );
        }
        return null;
      })()}

      {/* (Remove-from-album hover button retired — selection circle + sticky
          banner now provide remove/delete affordances.) */}
    </div>
  );
}
