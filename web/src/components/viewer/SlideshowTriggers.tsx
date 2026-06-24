/**
 * SlideshowTriggers — the "start slideshow" + "shuffle" header buttons.
 *
 * Replaces the identical inline button pair duplicated across all six
 * album-detail headers. Renders nothing when there are no playable photos
 * (previously each call site computed its own `hasPhotos` guard).
 */
import type { UseSlideshowResult } from "../../hooks/useSlideshow";

export default function SlideshowTriggers({ slideshow }: { slideshow: UseSlideshowResult }) {
  if (slideshow.totalSlides === 0) return null;
  return (
    <>
      <button
        onClick={() => slideshow.start(0)}
        className="text-fg-muted hover:text-accent-600 dark:hover:text-accent-400 transition-colors shrink-0"
        title="Start Slideshow"
      >
        <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24"><path d="M8 5v14l11-7z" /></svg>
      </button>
      <button
        onClick={() => { slideshow.toggleShuffle(); slideshow.start(0); }}
        className={`transition-colors shrink-0 ${slideshow.shuffleEnabled ? "text-accent-600 dark:text-accent-400" : "text-fg-muted hover:text-accent-600 dark:hover:text-accent-400"}`}
        title={slideshow.shuffleEnabled ? "Shuffle On" : "Shuffle Off"}
      >
        <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
          <path d="M10.59 9.17L5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41l-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z" />
        </svg>
      </button>
    </>
  );
}
