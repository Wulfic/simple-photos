/**
 * Hook for touch/swipe navigation gestures in the Viewer.
 *
 * Handles:
 *  - Horizontal swipe: prev/next photo navigation
 *  - Vertical swipe: up → info panel, down → close viewer / dismiss panel
 *  - Pinch-to-zoom (delegates zoom state from useZoomPan)
 *  - Double-tap-to-zoom (delegates zoom state from useZoomPan)
 *  - Panning while zoomed in
 */
import { useRef, useCallback } from "react";

interface UseSwipeNavigationParams {
  editMode: boolean;
  zoomScale: number;
  setZoomScale: (fn: (prev: number) => number) => void;
  setZoomOrigin: (origin: { x: number; y: number }) => void;
  panOffset: { x: number; y: number };
  setPanOffset: (offset: { x: number; y: number }) => void;
  pinchStartDist: React.MutableRefObject<number | null>;
  pinchStartScale: React.MutableRefObject<number>;
  panStart: React.MutableRefObject<{ x: number; y: number; ox: number; oy: number } | null>;
  lastTapTime: React.MutableRefObject<number>;
  viewerContainerRef: React.RefObject<HTMLDivElement | null>;
  goPrev: () => void;
  goNext: () => void;
  showInfoPanel: boolean;
  setShowInfoPanel: (v: boolean) => void;
  navigateBack: () => void;
}

export default function useSwipeNavigation({
  editMode,
  zoomScale,
  setZoomScale,
  setZoomOrigin,
  panOffset,
  setPanOffset,
  pinchStartDist,
  pinchStartScale,
  panStart,
  lastTapTime,
  viewerContainerRef,
  goPrev,
  goNext,
  showInfoPanel,
  setShowInfoPanel,
  navigateBack,
}: UseSwipeNavigationParams) {
  const touchStartX = useRef<number | null>(null);
  const touchStartY = useRef<number | null>(null);
  const swiped = useRef(false);

  const handleTouchStart = useCallback((e: React.TouchEvent) => {
    if (e.touches.length === 2) {
      // Pinch start
      const dx = e.touches[0].clientX - e.touches[1].clientX;
      const dy = e.touches[0].clientY - e.touches[1].clientY;
      pinchStartDist.current = Math.sqrt(dx * dx + dy * dy);
      pinchStartScale.current = zoomScale;
      // Set zoom origin to midpoint between fingers
      const rect = viewerContainerRef.current?.getBoundingClientRect();
      if (rect) {
        const mx = ((e.touches[0].clientX + e.touches[1].clientX) / 2 - rect.left) / rect.width * 100;
        const my = ((e.touches[0].clientY + e.touches[1].clientY) / 2 - rect.top) / rect.height * 100;
        setZoomOrigin({ x: mx, y: my });
      }
      swiped.current = true;
      return;
    }
    touchStartX.current = e.touches[0].clientX;
    touchStartY.current = e.touches[0].clientY;
    swiped.current = false;

    // Double-tap detection
    const now = Date.now();
    if (now - lastTapTime.current < 300 && !editMode) {
      const rect = viewerContainerRef.current?.getBoundingClientRect();
      if (rect) {
        if (zoomScale > 1) {
          setZoomScale(() => 1);
          setPanOffset({ x: 0, y: 0 });
        } else {
          const x = ((e.touches[0].clientX - rect.left) / rect.width) * 100;
          const y = ((e.touches[0].clientY - rect.top) / rect.height) * 100;
          setZoomOrigin({ x, y });
          setZoomScale(() => 2);
          setPanOffset({ x: 0, y: 0 });
        }
      }
      swiped.current = true;
      lastTapTime.current = 0;
      return;
    }
    lastTapTime.current = now;

    // Pan start when zoomed in
    if (zoomScale > 1) {
      panStart.current = { x: e.touches[0].clientX, y: e.touches[0].clientY, ox: panOffset.x, oy: panOffset.y };
    }
  }, [editMode, zoomScale, setZoomScale, setZoomOrigin, setPanOffset, panOffset, pinchStartDist, pinchStartScale, panStart, lastTapTime, viewerContainerRef]);

  const handleTouchMove = useCallback((e: React.TouchEvent) => {
    if (editMode) return;
    // Pinch-to-zoom
    if (e.touches.length === 2 && pinchStartDist.current !== null) {
      const dx = e.touches[0].clientX - e.touches[1].clientX;
      const dy = e.touches[0].clientY - e.touches[1].clientY;
      const dist = Math.sqrt(dx * dx + dy * dy);
      const ratio = dist / pinchStartDist.current;
      const newScale = Math.max(1, Math.min(5, pinchStartScale.current * ratio));
      setZoomScale(() => newScale);
      if (newScale <= 1) setPanOffset({ x: 0, y: 0 });
      return;
    }
    // Pan when zoomed
    if (zoomScale > 1 && panStart.current && e.touches.length === 1) {
      const dx = e.touches[0].clientX - panStart.current.x;
      const dy = e.touches[0].clientY - panStart.current.y;
      setPanOffset({ x: panStart.current.ox + dx, y: panStart.current.oy + dy });
      swiped.current = true;
    }
  }, [editMode, zoomScale, setZoomScale, setPanOffset, pinchStartDist, pinchStartScale, panStart]);

  const handleTouchEnd = useCallback((e: React.TouchEvent) => {
    if (editMode) return;
    const wasPinching = pinchStartDist.current !== null;
    pinchStartDist.current = null;
    panStart.current = null;

    // Snap back to normal mode when zoom reaches 1× (after pinch-out)
    if (zoomScale <= 1.05) {
      setZoomScale(() => 1);
      setPanOffset({ x: 0, y: 0 });
    }

    // If we were pinching and ended up back at 1×, allow next swipe gesture
    if (wasPinching && zoomScale <= 1.05) {
      swiped.current = false;
      touchStartX.current = null;
      touchStartY.current = null;
      return;
    }

    if (touchStartX.current === null || touchStartY.current === null || swiped.current) return;
    // Only allow swipe navigation when not zoomed in
    if (zoomScale > 1) return;
    const dx = e.changedTouches[0].clientX - touchStartX.current;
    const dy = e.changedTouches[0].clientY - touchStartY.current;
    const absDx = Math.abs(dx);
    const absDy = Math.abs(dy);
    // Require a minimum 50px horizontal swipe that's more horizontal than vertical
    if (absDx > 50 && absDx > absDy * 1.5) {
      swiped.current = true;
      if (dx > 0) goPrev();
      else goNext();
    }
    // Vertical swipe: up → info panel, down → close viewer
    else if (absDy > 80 && absDy > absDx * 1.5) {
      swiped.current = true;
      if (dy < 0) {
        setShowInfoPanel(true);
      } else {
        if (showInfoPanel) {
          setShowInfoPanel(false);
        } else {
          navigateBack();
        }
      }
    }
    touchStartX.current = null;
    touchStartY.current = null;
  }, [editMode, zoomScale, setZoomScale, setPanOffset, pinchStartDist, panStart, goPrev, goNext, showInfoPanel, setShowInfoPanel, navigateBack]);

  return {
    swiped,
    handleTouchStart,
    handleTouchMove,
    handleTouchEnd,
  };
}
