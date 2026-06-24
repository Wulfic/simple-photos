/**
 * SlideshowHost — renders the full-screen <Slideshow> overlay from a
 * `useSlideshow` result, or nothing when inactive.
 *
 * Replaces the 7 identical `{slideshow.isActive && <Slideshow …15 props… />}`
 * blocks that every viewer/album-detail screen used to hand-spread.
 */
import Slideshow from "./Slideshow";
import type { UseSlideshowResult } from "../../hooks/useSlideshow";

export default function SlideshowHost({ slideshow }: { slideshow: UseSlideshowResult }) {
  if (!slideshow.isActive) return null;
  return (
    <Slideshow
      currentBlobId={slideshow.currentBlobId}
      isPlaying={slideshow.isPlaying}
      currentSlide={slideshow.currentSlide}
      totalSlides={slideshow.totalSlides}
      shuffleEnabled={slideshow.shuffleEnabled}
      intervalMs={slideshow.intervalMs}
      transition={slideshow.transition}
      direction={slideshow.direction}
      onTogglePlay={slideshow.togglePlay}
      onNext={slideshow.next}
      onPrev={slideshow.prev}
      onToggleShuffle={slideshow.toggleShuffle}
      onSetSpeed={slideshow.setSpeed}
      onSetTransition={slideshow.setTransition}
      onExit={slideshow.stop}
    />
  );
}
