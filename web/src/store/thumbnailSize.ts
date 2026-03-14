import { create } from "zustand";

export type ThumbnailSize = "normal" | "large";

interface ThumbnailSizeState {
  thumbnailSize: ThumbnailSize;
  setThumbnailSize: (size: ThumbnailSize) => void;
  toggle: () => void;
  /** Returns Tailwind grid classes for the current thumbnail size */
  gridClasses: () => string;
}

/**
 * Zustand store for thumbnail size preference, persisted to localStorage.
 *
 * - "normal" → 3 columns on mobile (current default)
 * - "large"  → 2 columns on mobile (bigger thumbnails)
 */
export const useThumbnailSizeStore = create<ThumbnailSizeState>((set, get) => ({
  thumbnailSize:
    (localStorage.getItem("thumbnailSize") as ThumbnailSize) || "normal",

  /** Set the thumbnail size explicitly, persisting to localStorage. */
  setThumbnailSize: (size: ThumbnailSize) => {
    localStorage.setItem("thumbnailSize", size);
    set({ thumbnailSize: size });
  },

  /** Toggle between "normal" and "large" thumbnail sizes. */
  toggle: () => {
    const next = get().thumbnailSize === "normal" ? "large" : "normal";
    localStorage.setItem("thumbnailSize", next);
    set({ thumbnailSize: next });
  },

  gridClasses: () => {
    return get().thumbnailSize === "large"
      ? "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-2"
      : "grid grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2";
  },
}));
