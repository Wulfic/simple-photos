import { useState } from "react";
import type { CachedPhoto } from "../db";
import { ThumbnailImg } from "./AlbumTile";

export interface AddPhotosPanelProps {
  photos: CachedPhoto[];
  onAdd: (ids: string[]) => void;
  onCancel: () => void;
}

export default function AddPhotosPanel({ photos, onAdd, onCancel }: AddPhotosPanelProps) {
  const [selected, setSelected] = useState<Set<string>>(new Set());

  function toggle(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  return (
    <div className="mb-6 p-4 bg-accent-50 dark:bg-accent-900/30 rounded-lg">
      <div className="flex items-center justify-between mb-3">
        <p className="text-sm font-medium text-accent-800 dark:text-accent-300">
          Select photos to add ({selected.size} selected)
        </p>
        <div className="flex gap-2">
          <button
            onClick={() => onAdd(Array.from(selected))}
            disabled={selected.size === 0}
            className="btn btn-primary btn-sm"
          >
            Add Selected
          </button>
          <button
            onClick={onCancel}
            className="btn btn-secondary btn-sm"
          >
            Cancel
          </button>
        </div>
      </div>

      {photos.length === 0 ? (
        <p className="text-gray-700 dark:text-gray-400 text-sm">
          All photos are already in this album.
        </p>
      ) : (
        <div className="grid grid-cols-4 sm:grid-cols-6 md:grid-cols-8 gap-1 max-h-64 overflow-y-auto">
          {photos.map((photo) => {
            const isSelected = selected.has(photo.blobId);
            return (
              <div
                key={photo.blobId}
                className={`relative aspect-square rounded overflow-hidden cursor-pointer border-2 ${
                  isSelected ? "border-accent-600" : "border-transparent"
                }`}
                onClick={() => toggle(photo.blobId)}
              >
                <ThumbnailImg photo={photo} />
                {isSelected && (
                  <div className="absolute inset-0 bg-accent-600/30 flex items-center justify-center">
                    <svg
                      className="w-6 h-6 text-white"
                      fill="currentColor"
                      viewBox="0 0 20 20"
                    >
                      <path
                        fillRule="evenodd"
                        d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z"
                        clipRule="evenodd"
                      />
                    </svg>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
