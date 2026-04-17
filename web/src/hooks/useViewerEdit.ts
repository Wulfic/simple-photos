/**
 * Manages all edit-mode state for the Viewer: crop corners, brightness,
 * rotation, trim points, crop-zoom transform, and corner-drag handlers.
 */
import { useState, useRef, useCallback, useEffect } from "react";
import type { MediaType } from "../db";
import type { CropMetadata } from "../types/media";
import type { EditTab } from "../components/viewer/ViewerEditPanel";

export default function useViewerEdit(
  viewerContainerRef: React.RefObject<HTMLDivElement>,
) {
  // ── Edit mode state ────────────────────────────────────────────────────
  const [editMode, setEditMode] = useState(false);
  const [editTab, setEditTab] = useState<EditTab>("crop");
  const [cropData, setCropData] = useState<CropMetadata | null>(null);
  const [cropCorners, setCropCorners] = useState<{ x: number; y: number; w: number; h: number }>({ x: 0, y: 0, w: 1, h: 1 });
  const [draggingCorner, setDraggingCorner] = useState<string | null>(null);
  const [brightness, setBrightness] = useState(0);
  const [rotateValue, setRotateValue] = useState(0);
  const [trimStart, setTrimStart] = useState(0);
  const [trimEnd, setTrimEnd] = useState(0);
  const [mediaDuration, setMediaDuration] = useState(0);
  const [cropZoomStyle, setCropZoomStyle] = useState<React.CSSProperties>({});

  // ── Refs ────────────────────────────────────────────────────────────────
  const cropImageRef = useRef<HTMLImageElement>(null);
  const cropContainerRef = useRef<HTMLDivElement>(null);
  const audioRef = useRef<HTMLAudioElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const viewImgRef = useRef<HTMLImageElement>(null);

  // ── Reset (called when navigating to a new photo) ──────────────────────
  const resetEditState = useCallback(() => {
    setEditMode(false);
    setTrimStart(0);
    setTrimEnd(0);
    setMediaDuration(0);
    setBrightness(0);
    setRotateValue(0);
    setCropCorners({ x: 0, y: 0, w: 1, h: 1 });
    setEditTab("crop");
  }, []);

  // ── Rotation scale ─────────────────────────────────────────────────────
  /**
   * Compute CSS scale for 90°/270° rotation so the rotated element fits its
   * container without stretching.
   */
  const computeRotationScale = useCallback((el: HTMLElement | null, container: HTMLElement | null): number => {
    if (!el || !container) return 0.75; // safe fallback
    const containerW = container.clientWidth;
    const containerH = container.clientHeight;
    const elW = (el as HTMLVideoElement).videoWidth || (el as HTMLImageElement).naturalWidth || el.clientWidth;
    const elH = (el as HTMLVideoElement).videoHeight || (el as HTMLImageElement).naturalHeight || el.clientHeight;
    if (elW === 0 || elH === 0 || containerW === 0 || containerH === 0) return 0.75;
    const aspect = elW / elH;
    const containerAspect = containerW / containerH;
    let rendW: number, rendH: number;
    if (aspect > containerAspect) {
      rendW = containerW; rendH = containerW / aspect;
    } else {
      rendH = containerH; rendW = containerH * aspect;
    }
    return Math.min(containerW / rendH, containerH / rendW);
  }, []);

  // ── Crop zoom transform ────────────────────────────────────────────────
  const computeCropZoom = useCallback(() => {
    if (!cropData || editMode || !viewerContainerRef.current) {
      setCropZoomStyle({});
      return;
    }
    const el = viewImgRef.current ?? videoRef.current;
    if (!el) { setCropZoomStyle({}); return; }

    const container = viewerContainerRef.current;
    const vid = videoRef.current;
    const containerW = container.clientWidth;
    const containerH = container.clientHeight;
    let elW: number, elH: number;
    if (vid && vid === el && vid.videoWidth > 0 && vid.videoHeight > 0) {
      const aspect = vid.videoWidth / vid.videoHeight;
      if (aspect > containerW / containerH) {
        elW = containerW; elH = containerW / aspect;
      } else {
        elH = containerH; elW = containerH * aspect;
      }
    } else {
      const imgEl = el as HTMLImageElement;
      if (imgEl.naturalWidth > 0 && imgEl.naturalHeight > 0) {
        const aspect = imgEl.naturalWidth / imgEl.naturalHeight;
        if (aspect > containerW / containerH) {
          elW = containerW; elH = containerW / aspect;
        } else {
          elH = containerH; elW = containerH * aspect;
        }
      } else {
        elW = el.clientWidth;
        elH = el.clientHeight;
      }
    }
    if (elW === 0 || elH === 0 || containerW === 0 || containerH === 0) return;

    const rot = ((cropData.rotate ?? 0) % 360 + 360) % 360;

    console.log("[EDIT:cropZoom]", {
      cropData: { x: cropData.x, y: cropData.y, w: cropData.width, h: cropData.height,
                  rotate: cropData.rotate, brightness: cropData.brightness },
      containerW, containerH, elW, elH, rot,
    });

    // Full-frame crop (no actual cropping) with no rotation — just brightness.
    // Skip the translate/scale transform so the image stays at its natural
    // object-contain size instead of shrinking to 85%.
    const isFullFrame = cropData.x <= 0.01 && cropData.y <= 0.01 &&
                        cropData.width >= 0.99 && cropData.height >= 0.99;
    if (isFullFrame && rot === 0) {
      setCropZoomStyle({
        filter: cropData.brightness ? `brightness(${1 + (cropData.brightness ?? 0) / 100})` : undefined,
      });
      return;
    }

    const isSwapped = rot === 90 || rot === 270;
    const cropPixW = cropData.width * elW;
    const cropPixH = cropData.height * elH;
    const visW = isSwapped ? cropPixH : cropPixW;
    const visH = isSwapped ? cropPixW : cropPixH;
    const scaleW = containerW / visW;
    const scaleH = containerH / visH;
    const scale = Math.min(scaleW, scaleH) * 0.85;
    const cx = cropData.x + cropData.width / 2;
    const cy = cropData.y + cropData.height / 2;

    // Map crop center from content-normalized coords to element-% coords,
    // accounting for letterbox offsets introduced by object-contain on a
    // w-full h-full element whose aspect ratio differs from the container.
    const contentX = (containerW - elW) / 2;
    const contentY = (containerH - elH) / 2;
    const cxEl = (contentX + cx * elW) / containerW;
    const cyEl = (contentY + cy * elH) / containerH;

    setCropZoomStyle({
      transform: `translate(${(0.5 - cxEl) * 100}%, ${(0.5 - cyEl) * 100}%) scale(${scale})${rot ? ` rotate(${rot}deg)` : ""}`,
      transformOrigin: `${cxEl * 100}% ${cyEl * 100}%`,
      filter: cropData.brightness ? `brightness(${1 + (cropData.brightness ?? 0) / 100})` : undefined,
    });
  }, [cropData, editMode]);

  useEffect(() => {
    computeCropZoom();
    window.addEventListener("resize", computeCropZoom);
    return () => window.removeEventListener("resize", computeCropZoom);
  }, [computeCropZoom]);

  // ── Enter edit mode ────────────────────────────────────────────────────
  const enterEditMode = useCallback((mediaType: MediaType) => {
    let dur = mediaDuration;
    if (dur <= 0) {
      const el = audioRef.current ?? videoRef.current;
      if (el && isFinite(el.duration) && el.duration > 0) {
        dur = el.duration;
        setMediaDuration(dur);
      }
    }

    if (cropData) {
      setCropCorners({ x: cropData.x, y: cropData.y, w: cropData.width, h: cropData.height });
      setBrightness(cropData.brightness ?? 0);
      setRotateValue(cropData.rotate ?? 0);
      setTrimStart(cropData.trimStart ?? 0);
      setTrimEnd(cropData.trimEnd ?? dur);
    } else {
      setCropCorners({ x: 0, y: 0, w: 1, h: 1 });
      setBrightness(0);
      setRotateValue(0);
      setTrimStart(0);
      setTrimEnd(dur);
    }
    setEditTab(mediaType === "audio" || mediaType === "video" ? "trim" : "brightness");
    setEditMode(true);
  }, [cropData, mediaDuration]);

  // ── Corner drag handlers ───────────────────────────────────────────────
  const getMediaRect = useCallback((mediaType: MediaType) => {
    if (mediaType === "video" && videoRef.current) return videoRef.current.getBoundingClientRect();
    return cropImageRef.current?.getBoundingClientRect() ?? null;
  }, []);

  const handleCornerPointerDown = useCallback((corner: string) => {
    return (e: React.PointerEvent) => {
      e.preventDefault();
      e.stopPropagation();
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
      setDraggingCorner(corner);
    };
  }, []);

  const handleCornerPointerMove = useCallback((e: React.PointerEvent, mediaType: MediaType) => {
    if (!draggingCorner) return;
    const imgRect = getMediaRect(mediaType);
    if (!imgRect) return;
    const px = Math.max(0, Math.min(1, (e.clientX - imgRect.left) / imgRect.width));
    const py = Math.max(0, Math.min(1, (e.clientY - imgRect.top) / imgRect.height));
    setCropCorners((prev) => {
      const minSize = 0.05;
      let { x, y, w, h } = prev;
      if (draggingCorner === "tl") { const newX = Math.min(px, x + w - minSize); const newY = Math.min(py, y + h - minSize); w = w + (x - newX); h = h + (y - newY); x = newX; y = newY; }
      else if (draggingCorner === "tr") { const newR = Math.max(px, x + minSize); const newY = Math.min(py, y + h - minSize); w = newR - x; h = h + (y - newY); y = newY; }
      else if (draggingCorner === "bl") { const newX = Math.min(px, x + w - minSize); const newB = Math.max(py, y + minSize); w = w + (x - newX); x = newX; h = newB - y; }
      else if (draggingCorner === "br") { const newR = Math.max(px, x + minSize); const newB = Math.max(py, y + minSize); w = newR - x; h = newB - y; }
      return { x: Math.max(0, x), y: Math.max(0, y), w: Math.min(w, 1 - x), h: Math.min(h, 1 - y) };
    });
  }, [draggingCorner, getMediaRect]);

  const handleCornerPointerUp = useCallback(() => { setDraggingCorner(null); }, []);

  return {
    // State
    editMode, setEditMode,
    editTab, setEditTab,
    cropData, setCropData,
    cropCorners, setCropCorners,
    brightness, setBrightness,
    rotateValue, setRotateValue,
    trimStart, setTrimStart,
    trimEnd, setTrimEnd,
    mediaDuration, setMediaDuration,
    cropZoomStyle,
    // Refs
    cropImageRef, cropContainerRef, audioRef, videoRef, viewImgRef,
    // Functions
    resetEditState,
    computeRotationScale,
    computeCropZoom,
    enterEditMode,
    handleCornerPointerDown, handleCornerPointerMove, handleCornerPointerUp,
  };
}
