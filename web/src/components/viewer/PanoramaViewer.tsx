/**
 * PanoramaViewer — interactive panorama & 360° photo viewer.
 *
 * Two modes:
 *   - **Full view**: The entire panorama is visible (object-contain), scaled to fit
 *   - **Live view**: A viewport window that the user can pan through by dragging
 *     or using touch. For equirectangular (360°) photos, wraps around horizontally.
 *     For cylindrical panoramas, pans within the image bounds.
 *
 * Toggled via a pill button overlaid on the viewer.
 */
import { useEffect, useState, useRef, useCallback } from "react";

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

  // Pan state for live view
  const [panX, setPanX] = useState(0);
  const isDragging = useRef(false);
  const dragStartX = useRef(0);
  const panStartX = useRef(0);

  // Compute the viewport FOV — show ~90° of view for 360, or ~40% for pano
  const fovRatio = subtype === "equirectangular" ? 0.25 : 0.4;

  // Reset pan when switching modes or image changes
  useEffect(() => {
    setPanX(0);
  }, [mediaUrl, mode]);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (mode !== "live") return;
    isDragging.current = true;
    dragStartX.current = e.clientX;
    panStartX.current = panX;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, [mode, panX]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!isDragging.current || mode !== "live") return;
    const dx = e.clientX - dragStartX.current;
    const container = containerRef.current;
    if (!container) return;

    const containerWidth = container.clientWidth;
    // Scale drag to image-space: how much of the image width to move per pixel
    const scaledWidth = imageWidth * (1 / fovRatio);
    const dxNorm = (dx / containerWidth) * scaledWidth;

    let newPan = panStartX.current + dxNorm;

    if (subtype === "equirectangular") {
      // Wrap around for 360°
      newPan = ((newPan % imageWidth) + imageWidth) % imageWidth;
    } else {
      // Clamp for panorama
      const maxPan = imageWidth - imageWidth * fovRatio;
      newPan = Math.max(0, Math.min(maxPan, newPan));
    }

    setPanX(newPan);
  }, [mode, imageWidth, fovRatio, subtype]);

  const handlePointerUp = useCallback(() => {
    isDragging.current = false;
  }, []);

  // Touch handling for mobile swipe
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
    const container = containerRef.current;
    if (!container) return;

    const containerWidth = container.clientWidth;
    const scaledWidth = imageWidth * (1 / fovRatio);
    const dxNorm = (dx / containerWidth) * scaledWidth;
    let newPan = touchPanStart.current + dxNorm;

    if (subtype === "equirectangular") {
      newPan = ((newPan % imageWidth) + imageWidth) % imageWidth;
    } else {
      const maxPan = imageWidth - imageWidth * fovRatio;
      newPan = Math.max(0, Math.min(maxPan, newPan));
    }
    setPanX(newPan);
  }, [mode, imageWidth, fovRatio, subtype]);

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

  // Live view: crop to a viewport window
  const viewportFraction = fovRatio;
  // Scale the image so the viewport fraction fills the container
  const scale = 1 / viewportFraction;
  // Translate to show the right portion
  const translateXPercent = -(panX / imageWidth) * 100 * scale;

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
      <img
        ref={imgRef}
        src={mediaUrl}
        alt="Panorama"
        className="absolute top-1/2 h-full"
        style={{
          transform: `translateY(-50%) translateX(${translateXPercent}%) scaleX(${scale})`,
          transformOrigin: "left center",
          width: `${100 * scale}%`,
          objectFit: "cover",
          pointerEvents: "none",
        }}
        draggable={false}
      />
      {/* Pan position indicator */}
      <div className="absolute bottom-28 left-1/2 -translate-x-1/2 z-20">
        <div className="bg-black/50 rounded-full h-1 w-32 overflow-hidden">
          <div
            className="bg-white h-full rounded-full transition-all duration-75"
            style={{
              width: `${viewportFraction * 100}%`,
              marginLeft: `${(panX / imageWidth) * 100}%`,
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
