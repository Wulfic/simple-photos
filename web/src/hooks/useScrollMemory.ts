/**
 * useScrollMemory — remembers the window scroll offset per "key" (usually the
 * route pathname) and restores it when the page re-mounts.
 *
 * Why this exists: the gallery and album grids scroll the document. Opening a
 * photo swaps the route's <Outlet> from the grid page to the full-screen
 * <Viewer>, which unmounts the grid. Coming back (the viewer's origin-aware
 * `navigate(backTo)` is a PUSH, so the browser's native scroll restoration does
 * NOT kick in) re-mounts the grid at the top — losing the user's place after
 * they may have scrolled past thousands of photos.
 *
 * The grid height is established asynchronously (live-query photos resolve, then
 * JustifiedGrid measures its width via ResizeObserver and lays out rows), so a
 * single scrollTo on mount races the layout and clamps short. We therefore
 * restore on a requestAnimationFrame loop, re-applying the target each frame
 * until the document is tall enough to reach it (or a short budget expires).
 *
 * @param key   Stable identity for the scrollable view (e.g. location.pathname).
 * @param ready Flips true once the grid has rendered items — gates restoration
 *              so we don't try to scroll an empty page.
 */
import { useEffect, useRef } from "react";

// Module-level so positions survive component unmount (grid → viewer → grid).
const scrollPositions = new Map<string, number>();

// ~1s at 60fps — enough for live-query + grid layout to settle, bounded so we
// never spin forever if the content genuinely shrank below the saved offset.
const MAX_RESTORE_FRAMES = 60;

export function useScrollMemory(key: string, ready: boolean) {
  // The offset still owed for this key on the current mount. `undefined` means
  // "nothing to restore — resume saving immediately". Recomputed when the key
  // changes (e.g. navigating album A → album B without a full unmount).
  const pendingRef = useRef<number | undefined>(undefined);
  const lastKeyRef = useRef<string | null>(null);
  if (lastKeyRef.current !== key) {
    lastKeyRef.current = key;
    const saved = scrollPositions.get(key);
    pendingRef.current = saved && saved > 0 ? saved : undefined;
  }

  // Continuously remember the latest offset for this key. While a restoration
  // is still pending we ignore scroll events so the browser's reset-to-top on
  // mount (and our own scrollTo retries) don't clobber the remembered value.
  useEffect(() => {
    const onScroll = () => {
      if (pendingRef.current !== undefined) return;
      scrollPositions.set(key, window.scrollY);
    };
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, [key]);

  // Restore once the grid is ready, retrying across frames until the page is
  // tall enough to honour the saved offset.
  useEffect(() => {
    if (!ready) return;
    const target = pendingRef.current;
    if (target === undefined) return;

    let frame = 0;
    let attempts = 0;
    const step = () => {
      const maxScroll = document.documentElement.scrollHeight - window.innerHeight;
      window.scrollTo(0, Math.min(target, Math.max(maxScroll, 0)));
      attempts += 1;
      if (maxScroll >= target || attempts >= MAX_RESTORE_FRAMES) {
        // Done — clear the pending flag so scroll saving resumes.
        pendingRef.current = undefined;
        return;
      }
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [key, ready]);
}
