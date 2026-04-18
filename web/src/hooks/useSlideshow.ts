/**
 * Hook that manages slideshow playback state.
 *
 * Handles play/pause, auto-advance timer, shuffle, speed selection,
 * transition preference, and photo-only filtering.
 * Persists preferences (speed, shuffle, transition) in localStorage.
 */
import { useState, useCallback, useEffect, useRef, useMemo } from "react";

export type SlideshowTransition = "fade" | "slide" | "zoom" | "dissolve";

const STORAGE_KEYS = {
  speed: "slideshow_speed",
  shuffle: "slideshow_shuffle",
  transition: "slideshow_transition",
};

function loadPref<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    if (v === null) return fallback;
    return JSON.parse(v);
  } catch {
    return fallback;
  }
}

/** Fisher-Yates shuffle (returns new array). */
function shuffleArray(arr: number[]): number[] {
  const a = [...arr];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

export interface UseSlideshowResult {
  /** Whether the slideshow overlay is active. */
  isActive: boolean;
  /** Whether auto-advance is playing. */
  isPlaying: boolean;
  /** Index into filteredPhotoIds (the slideshow-local index). */
  currentSlide: number;
  /** Total number of slides. */
  totalSlides: number;
  /** The blob ID of the current slide. */
  currentBlobId: string | undefined;
  /** Shuffle toggle state. */
  shuffleEnabled: boolean;
  /** Auto-advance interval in ms. */
  intervalMs: number;
  /** Active transition effect. */
  transition: SlideshowTransition;
  /** Direction of the current transition (+1 forward, -1 backward). */
  direction: 1 | -1;

  start: (startIndex?: number) => void;
  stop: () => void;
  togglePlay: () => void;
  next: () => void;
  prev: () => void;
  toggleShuffle: () => void;
  setSpeed: (ms: number) => void;
  setTransition: (t: SlideshowTransition) => void;
}

/**
 * @param allBlobIds  Full list of blob IDs (may include videos/audio).
 * @param mediaTypes  Map of blobId → mediaType for filtering.
 */
export default function useSlideshow(
  allBlobIds: string[] | undefined,
  mediaTypes: Map<string, string>,
): UseSlideshowResult {
  const [isActive, setIsActive] = useState(false);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentSlide, setCurrentSlide] = useState(0);
  const [shuffleEnabled, setShuffleEnabled] = useState(() => loadPref(STORAGE_KEYS.shuffle, false));
  const [intervalMs, setIntervalMs] = useState(() => loadPref(STORAGE_KEYS.speed, 5000));
  const [transition, setTransitionState] = useState<SlideshowTransition>(
    () => loadPref(STORAGE_KEYS.transition, "fade"),
  );
  const [direction, setDirection] = useState<1 | -1>(1);
  const shuffledOrder = useRef<number[]>([]);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Filter to photos only (skip video, audio).
  const filteredPhotoIds = useMemo(() => {
    if (!allBlobIds) return [];
    return allBlobIds.filter((id) => {
      const mt = mediaTypes.get(id);
      return !mt || mt === "photo" || mt === "gif";
    });
  }, [allBlobIds, mediaTypes]);

  const totalSlides = filteredPhotoIds.length;

  // Build the display order (sequential or shuffled).
  const displayOrder = useMemo(() => {
    const seq = Array.from({ length: totalSlides }, (_, i) => i);
    if (shuffleEnabled) {
      shuffledOrder.current = shuffleArray(seq);
      return shuffledOrder.current;
    }
    shuffledOrder.current = seq;
    return seq;
  }, [totalSlides, shuffleEnabled]);

  const resolvedIndex = displayOrder[currentSlide] ?? 0;
  const currentBlobId = filteredPhotoIds[resolvedIndex];

  // ── Timer management ──────────────────────────────────────────────────

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const startTimer = useCallback(() => {
    clearTimer();
    timerRef.current = setInterval(() => {
      setDirection(1);
      setCurrentSlide((prev) => (prev + 1) % totalSlides);
    }, intervalMs);
  }, [clearTimer, intervalMs, totalSlides]);

  // Restart timer when speed changes while playing.
  useEffect(() => {
    if (isPlaying && isActive) startTimer();
    return clearTimer;
  }, [isPlaying, isActive, startTimer, clearTimer]);

  // ── Public API ────────────────────────────────────────────────────────

  const start = useCallback((startIndex?: number) => {
    if (totalSlides === 0) return;
    // Map the gallery index to a slideshow index.
    let slideIdx = 0;
    if (startIndex !== undefined) {
      const targetBlobId = allBlobIds?.[startIndex];
      if (targetBlobId) {
        const photoIdx = filteredPhotoIds.indexOf(targetBlobId);
        if (photoIdx >= 0) {
          slideIdx = displayOrder.indexOf(photoIdx);
          if (slideIdx < 0) slideIdx = 0;
        }
      }
    }
    setCurrentSlide(slideIdx);
    setDirection(1);
    setIsActive(true);
    setIsPlaying(true);
  }, [totalSlides, allBlobIds, filteredPhotoIds, displayOrder]);

  const stop = useCallback(() => {
    setIsActive(false);
    setIsPlaying(false);
    clearTimer();
  }, [clearTimer]);

  const togglePlay = useCallback(() => {
    setIsPlaying((p) => !p);
  }, []);

  const next = useCallback(() => {
    setDirection(1);
    setCurrentSlide((prev) => (prev + 1) % totalSlides);
    // Reset timer on manual advance.
    if (isPlaying) startTimer();
  }, [totalSlides, isPlaying, startTimer]);

  const prev = useCallback(() => {
    setDirection(-1);
    setCurrentSlide((prev) => (prev - 1 + totalSlides) % totalSlides);
    if (isPlaying) startTimer();
  }, [totalSlides, isPlaying, startTimer]);

  const toggleShuffle = useCallback(() => {
    setShuffleEnabled((s) => {
      const v = !s;
      localStorage.setItem(STORAGE_KEYS.shuffle, JSON.stringify(v));
      return v;
    });
  }, []);

  const setSpeed = useCallback((ms: number) => {
    setIntervalMs(ms);
    localStorage.setItem(STORAGE_KEYS.speed, JSON.stringify(ms));
  }, []);

  const setTransition = useCallback((t: SlideshowTransition) => {
    setTransitionState(t);
    localStorage.setItem(STORAGE_KEYS.transition, JSON.stringify(t));
  }, []);

  return {
    isActive,
    isPlaying,
    currentSlide,
    totalSlides,
    currentBlobId,
    shuffleEnabled,
    intervalMs,
    transition,
    direction,
    start,
    stop,
    togglePlay,
    next,
    prev,
    toggleShuffle,
    setSpeed,
    setTransition,
  };
}
