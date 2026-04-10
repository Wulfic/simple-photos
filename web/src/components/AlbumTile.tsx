import { useEffect, useState, useRef } from "react";
import type { CachedPhoto } from "../db";

// ── Thumbnail helper ──────────────────────────────────────────────────────────

export function ThumbnailImg({ photo }: { photo: CachedPhoto }) {
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    if (photo.thumbnailData) {
      // Encrypted thumbnail stored in IndexedDB
      const mime = photo.thumbnailMimeType || (photo.mediaType === "gif" ? "image/gif" : "image/jpeg");
      const url = URL.createObjectURL(
        new Blob([photo.thumbnailData], { type: mime })
      );
      setSrc(url);
      return () => URL.revokeObjectURL(url);
    } else {
      setSrc(null);
    }
  }, [photo.thumbnailData, photo.thumbnailMimeType, photo.mediaType]);

  if (src) {
    return (
      <img
        src={src}
        alt={photo.filename}
        className="w-full h-full object-cover"
        loading="lazy"
      />
    );
  }

  return (
    <div className="w-full h-full flex items-center justify-center text-gray-400 text-xs px-1 text-center bg-gray-100 dark:bg-gray-700">
      {photo.filename}
    </div>
  );
}

// ── Utility ───────────────────────────────────────────────────────────────────

function formatDuration(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
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
      className={`relative w-full h-full bg-gray-100 dark:bg-gray-700 rounded overflow-hidden cursor-pointer group ${
        isSelected ? "ring-2 ring-blue-500" : ""
      }`}
      onPointerDown={handlePointerDown}
      onPointerUp={handlePointerUp}
      onPointerLeave={handlePointerLeave}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="w-full h-full">
        <ThumbnailImg photo={photo} />
      </div>

      {/* Selection circle */}
      {isSelectionMode && (
        <div className={`absolute top-1.5 right-1.5 w-6 h-6 rounded-full border-2 flex items-center justify-center ${
          isSelected
            ? "bg-green-500 border-green-500"
            : "bg-white/80 border-gray-400/50"
        }`}>
          {isSelected && (
            <svg className="w-4 h-4 text-white" fill="currentColor" viewBox="0 0 20 20">
              <path fillRule="evenodd" d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z" clipRule="evenodd" />
            </svg>
          )}
        </div>
      )}

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

      {/* Remove button on hover (only when NOT in selection mode) */}
      {!isSelectionMode && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          onPointerDown={(e) => e.stopPropagation()}
          className="absolute top-1 right-1 bg-red-600 text-white rounded-full w-6 h-6 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity text-xs"
          title="Remove from album"
        >
          ×
        </button>
      )}
    </div>
  );
}
