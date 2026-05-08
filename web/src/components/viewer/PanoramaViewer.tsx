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
 * The flat live view does NOT apply any horizontal CSS scaling — that
 * previously stretched the image and clipped both ends of the visible
 * range.  Instead we size the image at its true aspect ratio relative to
 * the container and translate it in pixels.
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

  // Pan offset in *rendered-image* pixels.  0 = left edge of image aligned
  // with left edge of viewport.
  const [panX, setPanX] = useState(0);
  const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });

  // Track container size so we can compute rendered image width.
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
    setPanX(0);
  }, [mediaUrl, mode]);

  // Rendered image dimensions when displayed at h-full preserving aspect.
  const aspect = imageWidth > 0 && imageHeight > 0 ? imageWidth / imageHeight : 1;
  const renderedHeight = containerSize.height || 1;
  const renderedWidth = renderedHeight * aspect;
  const maxPan = Math.max(0, renderedWidth - containerSize.width);

  const clampPan = useCallback(
    (next: number): number => Math.max(0, Math.min(maxPan, next)),
    [maxPan],
  );

  // Pointer drag.
  const isDragging = useRef(false);
  const dragStartX = useRef(0);
  const panStartX = useRef(0);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (mode !== "live") return;
    isDragging.current = true;
    dragStartX.current = e.clientX;
    panStartX.current = panX;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, [mode, panX]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!isDragging.current || mode !== "live") return;
    // Drag right ⇒ image moves right ⇒ panX decreases.
    const dx = e.clientX - dragStartX.current;
    setPanX(clampPan(panStartX.current - dx));
  }, [mode, clampPan]);

  const handlePointerUp = useCallback(() => {
    isDragging.current = false;
  }, []);

  // Touch handling for mobile swipe.
  const touchStartX = useRef(0);
  const touchPanStart = useRef(0);

  const handleTouchStart = useCallback((e: React.TouchEvent) => {
    if (mode !== "live" || e.touches.length !== 1) return;
    touchStartX.current = e.touches[0].clientX;
    touchPanStart.current = panX;
  }, [mode, panX]);

  const handleTouchMove = useCallback((e: React.TouchEvent) => {
    if (mode !== "live" || e.touches.length !== 1) return;
    e.preventDefault();
    const dx = e.touches[0].clientX - touchStartX.current;
    setPanX(clampPan(touchPanStart.current - dx));
  }, [mode, clampPan]);

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

  // ── Live view, cylindrical panorama: flat scrollable viewport ────────
  // The image is rendered at its natural aspect ratio with height matching
  // the container, then translated in screen pixels.  No scaleX hacks —
  // the image is never stretched.
  const tx = -panX;
  const indicatorPosition = renderedWidth > 0 ? panX / renderedWidth : 0;
  const indicatorWidth = renderedWidth > 0
    ? Math.min(1, containerSize.width / renderedWidth)
    : 0;

  return (
    <div
      ref={containerRef}
      className="relative w-full h-full overflow-hidden cursor-grab active:cursor-grabbing select-none"
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerUp}
      onTouchStart={handleTouchStart}
      onTouchMove={handleTouchMove}
    >
      <div
        className="absolute top-0 left-0 h-full"
        style={{
          width: renderedWidth ? `${renderedWidth}px` : "auto",
          transform: `translate3d(${tx}px, 0, 0)`,
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
