/** Zustand store for gallery thumbnail size preference (persists to localStorage). */
import { create } from "zustand";

export type ThumbnailSize = "normal" | "large";

interface ThumbnailSizeState {
  thumbnailSize: ThumbnailSize;
  setThumbnailSize: (size: ThumbnailSize) => void;
  toggle: () => void;
  /** Returns Tailwind grid classes (used as fallback for non-justified grids). */
  gridClasses: () => string;
  /** Target row height in pixels for the justified flex-row grid layout. */
  targetRowHeight: () => number;
}

/**
 * Zustand store for thumbnail size preference, persisted to localStorage.
 *
 * - "normal" → ~180 px row height (more photos visible per screen)
 * - "large"  → ~280 px row height (bigger previews, fewer per screen)
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

  targetRowHeight: () => {
    return get().thumbnailSize === "large" ? 280 : 180;
  },
}));
