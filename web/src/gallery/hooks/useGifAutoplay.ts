/**
 * GIF autoplay hook — extracted from MediaTile + SecureGalleryTile.
 *
 * Manages the IntersectionObserver lifecycle for large GIFs that need
 * full-blob loading for animation (their thumbnail is a static JPEG).
 *
 * State machine: idle → loading → playing → paused → error
 *
 * When the tile scrolls into view the full animated GIF is loaded via
 * `loadFullGif()`.  When it scrolls out, the display swaps back to the
 * static thumbnail to save memory.
 */
import { useState, useEffect, useRef, type RefObject } from "react";
import { loadFullGif } from "../../utils/gifLoader";

export type GifAutoplayState = "idle" | "loading" | "playing" | "paused" | "error";

export interface GifAutoplayResult {
  /** Full animated GIF blob URL (null when not loaded or paused) */
  fullGifUrl: string | null;
  /** Current autoplay state */
  state: GifAutoplayState;
  /** Whether the tile is currently in the viewport */
  inView: boolean;
}

/**
 * @param tileRef   - Ref to the tile's root DOM element (for IntersectionObserver)
 * @param blobId    - The storage blob ID to load the full GIF from
 * @param serverPhotoId - Server photo ID (for server-side GIFs)
 * @param enabled   - Set to false for GIFs that already have animated thumbnails
 * @param rootMargin - IntersectionObserver root margin (default "200px")
 */
export function useGifAutoplay(
  tileRef: RefObject<HTMLDivElement | null>,
  blobId: string | undefined,
  serverPhotoId: string | undefined,
  enabled: boolean,
  rootMargin = `${Math.max(200, Math.round(typeof window !== "undefined" ? window.innerHeight * 0.5 : 200))}px`,
): GifAutoplayResult {
  const [state, setState] = useState<GifAutoplayState>("idle");
  const [inView, setInView] = useState(false);
  const fullGifUrlRef = useRef<string | null>(null);
  const [fullGifUrl, setFullGifUrl] = useState<string | null>(null);

  // IntersectionObserver for viewport tracking
  useEffect(() => {
    if (!enabled) return;
    const el = tileRef.current;
    if (!el) return;

    const observer = new IntersectionObserver(
      ([entry]) => {
        setInView(entry.isIntersecting);
      },
      { rootMargin },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [enabled, rootMargin, tileRef]);

  // Load full GIF when scrolling into view
  useEffect(() => {
    if (!enabled || !inView || fullGifUrlRef.current || !blobId) return;

    let cancelled = false;
    setState("loading");

    loadFullGif(blobId, serverPhotoId).then((url) => {
      if (cancelled) return;
      if (url) {
        fullGifUrlRef.current = url;
        setFullGifUrl(url);
        setState("playing");
      } else {
        setState("error");
      }
    });

    return () => { cancelled = true; };
  }, [enabled, inView, blobId, serverPhotoId]);

  // Swap between full GIF (in view) and null (out of view → caller shows thumbnail)
  useEffect(() => {
    if (!enabled || !fullGifUrlRef.current) return;
    if (inView) {
      setFullGifUrl(fullGifUrlRef.current);
      setState("playing");
    } else {
      setFullGifUrl(null);
      setState("paused");
    }
  }, [enabled, inView]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (fullGifUrlRef.current) {
        URL.revokeObjectURL(fullGifUrlRef.current);
        fullGifUrlRef.current = null;
      }
    };
  }, []);

  if (!enabled) {
    return { fullGifUrl: null, state: "idle", inView: false };
  }

  return { fullGifUrl, state, inView };
}
