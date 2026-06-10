/**
 * PanoramaViewer — interactive panorama & 360° photo viewer.
 *
 * Two modes:
 *   - **Full view**: The entire panorama is visible (object-contain), scaled to fit.
 *   - **Live view**:
 *       - For `equirectangular` (true 360° photo sphere) → renders a WebGL
 *         photo-sphere via `Sphere360Viewer` (drag to look around, pinch /
 *         wheel to zoom).
 *       - For `panorama` (cylindrical / wide stitch) → renders a flat
 *         scrollable viewport at the image's natural aspect ratio.
 *
 * The flat live view pans along the image's long axis: horizontal stitches
 * pan left/right, vertical panoramas (h/w ≥ 2.5, e.g. Samsung vertical
 * pano) pan up/down — previously they got the horizontal-only viewer,
 * which computed a 0-pixel pan range and did nothing.
 *
 * Input handling uses pointer events exclusively (they cover mouse, pen,
 * and touch).  The old parallel touch handlers called `preventDefault()`
 * inside React's passive listeners — a no-op that let the page scroll
 * instead of panning on mobile.  `touch-action: none` on the live viewport
 * is what actually stops the browser from hijacking the gesture.
 */
import { useEffect, useState, useRef, useCallback, useLayoutEffect } from "react";
import Sphere360Viewer from "./Sphere360Viewer";

type ViewMode = "full" | "live";

interface PanoramaViewerProps {
  /** Object URL or data URL of the panorama image */
  mediaUrl: string;
  /** "panorama" (cylindrical, bounded pan) or "equirectangular" (360° wrap) */
  subtype: "panorama" | "equirectangular";
  /** Natural image width */
  imageWidth: number;
  /** Natural image height */
  imageHeight: number;
}

export default function PanoramaViewer({ mediaUrl, subtype, imageWidth, imageHeight }: PanoramaViewerProps) {
  const [mode, setMode] = useState<ViewMode>("full");
  const containerRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);

  // Pan offset in *rendered-image* pixels along the pan axis.
  // 0 = leading edge of image aligned with leading edge of viewport.
  const [pan, setPan] = useState(0);
  const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });

  // Track container size so we can compute rendered image dimensions.
  useLayoutEffect(() => {
    if (mode !== "live") return;
    const el = containerRef.current;
    if (!el) return;
    const update = () => setContainerSize({ width: el.clientWidth, height: el.clientHeight });
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [mode]);

  // Reset pan when switching modes or image changes.
  useEffect(() => {
    setPan(0);
  }, [mediaUrl, mode]);

  const aspect = imageWidth > 0 && imageHeight > 0 ? imageWidth / imageHeight : 1;
  // Pan along the image's long axis.
  const horizontal = aspect >= 1;

  // Rendered dimensions: the short axis fills the container, the long axis
  // overflows and is translated.
  const renderedWidth = horizontal
    ? (containerSize.height || 1) * aspect
    : containerSize.width || 1;
  const renderedHeight = horizontal
    ? containerSize.height || 1
    : (containerSize.width || 1) / aspect;
  const viewportSpan = horizontal ? containerSize.width : containerSize.height;
  const renderedSpan = horizontal ? renderedWidth : renderedHeight;
  const maxPan = Math.max(0, renderedSpan - viewportSpan);

  const clampPan = useCallback(
    (next: number): number => Math.max(0, Math.min(maxPan, next)),
    [maxPan],
  );

  // Pointer drag (mouse, pen, and touch — touch-action: none makes touch
  // pointer events fire reliably without page scroll).
  const isDragging = useRef(false);
  const dragStart = useRef(0);
  const panStart = useRef(0);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (mode !== "live") return;
    isDragging.current = true;
    dragStart.current = horizontal ? e.clientX : e.clientY;
    panStart.current = pan;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, [mode, pan, horizontal]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!isDragging.current || mode !== "live") return;
    // Drag toward positive screen axis ⇒ image follows ⇒ pan decreases.
    const delta = (horizontal ? e.clientX : e.clientY) - dragStart.current;
    setPan(clampPan(panStart.current - delta));
  }, [mode, clampPan, horizontal]);

  const handlePointerUp = useCallback((e: React.PointerEvent) => {
    isDragging.current = false;
    try {
      (e.target as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* pointer capture may already be released */
    }
  }, []);

  if (mode === "full") {
    return (
      <div className="relative w-full h-full flex items-center justify-center">
        <img
          ref={imgRef}
          src={mediaUrl}
          alt="Panorama"
          className="w-full h-full object-contain"
        />
        <ModeToggle mode={mode} subtype={subtype} onToggle={() => setMode("live")} />
      </div>
    );
  }

  // ── Live view, equirectangular: real WebGL photo sphere ──────────────
  if (subtype === "equirectangular") {
    return <Sphere360Viewer mediaUrl={mediaUrl} onExitToFull={() => setMode("full")} />;
  }

  // ── Live view, flat panorama: scrollable viewport along the long axis ─
  // The image is rendered at its natural aspect ratio with the short side
  // matching the container, then translated in screen pixels.  No scale
  // hacks — the image is never stretched.
  const tx = horizontal ? -pan : 0;
  const ty = horizontal ? 0 : -pan;
  const indicatorPosition = renderedSpan > 0 ? pan / renderedSpan : 0;
  const indicatorWidth = renderedSpan > 0 ? Math.min(1, viewportSpan / renderedSpan) : 0;

  return (
    <div
      ref={containerRef}
      className="relative w-full h-full overflow-hidden cursor-grab active:cursor-grabbing select-none"
      style={{ touchAction: "none" }}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerUp}
    >
      <div
        className="absolute top-0 left-0"
        style={{
          width: renderedWidth ? `${renderedWidth}px` : "auto",
          height: renderedHeight ? `${renderedHeight}px` : "100%",
          transform: `translate3d(${tx}px, ${ty}px, 0)`,
          willChange: "transform",
        }}
      >
        <img
          ref={imgRef}
          src={mediaUrl}
          alt="Panorama"
          className="block h-full w-full"
          style={{ pointerEvents: "none", objectFit: "fill" }}
          draggable={false}
        />
      </div>

      {/* Pan position indicator */}
      <div className="absolute bottom-28 left-1/2 -translate-x-1/2 z-20 pointer-events-none">
        <div className="bg-black/50 rounded-full h-1 w-32 overflow-hidden">
          <div
            className="bg-white h-full rounded-full transition-all duration-75"
            style={{
              width: `${indicatorWidth * 100}%`,
              marginLeft: `${indicatorPosition * 100}%`,
            }}
          />
        </div>
      </div>
      <ModeToggle mode={mode} subtype={subtype} onToggle={() => setMode("full")} />
    </div>
  );
}

/** Mode toggle pill button */
function ModeToggle({
  mode,
  subtype,
  onToggle,
}: {
  mode: ViewMode;
  subtype: "panorama" | "equirectangular";
  onToggle: () => void;
}) {
  const label = mode === "full" ? "Live View" : "Full View";
  const icon = subtype === "equirectangular" ? "360°" : "PANO";

  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        onToggle();
      }}
      className="absolute bottom-24 left-1/2 -translate-x-1/2 z-30 flex items-center gap-2 px-4 py-1.5 rounded-full bg-black/60 text-white text-sm font-medium hover:bg-black/80 transition-colors backdrop-blur-sm"
    >
      <span className="text-xs font-bold opacity-70">{icon}</span>
      <span>{label}</span>
    </button>
  );
}
