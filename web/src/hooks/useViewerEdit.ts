/**
 * Manages all edit-mode state for the Viewer: crop corners, brightness,
 * rotation, trim points, crop-zoom transform, and corner-drag handlers.
 */
import { useState, useRef, useCallback, useEffect } from "react";
import type { MediaType } from "../db";
import type { CropMetadata } from "../types/media";
import type { EditTab } from "../components/viewer/ViewerEditPanel";

/**
 * Inset factor applied to the photo when the crop tab is active.
 *
 * Without an inset the photo fills the entire viewer and the corner
 * grab-handles end up flush against (or past) the screen edge, where
 * they're nearly impossible to grab — especially on touch.  Scaling
 * the photo down to ~85% of the container leaves a visible gutter
 * around it so the user can always drag the corners inward and outward.
 *
 * The factor is purely visual: it does NOT affect crop fractions, which
 * remain expressed in the rotated photo's own 0–1 coordinate space.
 */
export const EDIT_CROP_PADDING_SCALE = 0.85;

/**
 * Build the CSS that makes an object-contain `<img>` (sized `elW × elH`,
 * letterboxed inside a `containerW × containerH` box) display ONLY the crop
 * rectangle, rotated upright and fit to the container (object-contain
 * semantics).
 *
 * The crop fractions (x, y, w, h) are expressed in the *rotated* frame — the
 * frame the user draws on. We map that rect onto the un-rotated element (a 90°
 * multiple keeps it axis-aligned) for the clip-path, place the crop centre at
 * the container centre, and scale to fit. Shared by the saved-photo view
 * (`computeCropZoom`) and the in-editor preview so both stay pixel-consistent.
 *
 * Verified numerically for full-frame + asymmetric crops at 0/90/180/270.
 */
export function cropFitStyle(
  crop: { x: number; y: number; w: number; h: number },
  rotate: number,
  brightness: number | undefined,
  elW: number,
  elH: number,
  containerW: number,
  containerH: number,
): React.CSSProperties {
  const filter = brightness ? `brightness(${1 + brightness / 100})` : undefined;
  const rot = ((rotate % 360) + 360) % 360;
  const { x, y, w, h } = crop;

  // Full-frame, no rotation → leave the image at its natural contain size.
  const isFullFrame = x <= 0.01 && y <= 0.01 && w >= 0.99 && h >= 0.99;
  if (isFullFrame && rot === 0) return { filter };

  const isSwapped = rot === 90 || rot === 270;
  const cp = x + w / 2;
  const cq = y + h / 2;

  // Crop footprint AFTER rotation, in element px → fit-to-viewport scale.
  const footW = isSwapped ? w * elH : w * elW;
  const footH = isSwapped ? h * elW : h * elH;
  const scale = Math.min(containerW / footW, containerH / footH);

  // (a,b) = crop centre, and [aMin,aMax]×[bMin,bMax] = crop rect, both in the
  // UN-rotated element's 0–1 space (per rotation).
  let a: number, b: number, aMin: number, aMax: number, bMin: number, bMax: number;
  if (rot === 90) {
    a = cq; b = 1 - cp;
    aMin = y; aMax = y + h; bMin = 1 - (x + w); bMax = 1 - x;
  } else if (rot === 180) {
    a = 1 - cp; b = 1 - cq;
    aMin = 1 - (x + w); aMax = 1 - x; bMin = 1 - (y + h); bMax = 1 - y;
  } else if (rot === 270) {
    a = 1 - cq; b = cp;
    aMin = 1 - (y + h); aMax = 1 - y; bMin = x; bMax = x + w;
  } else {
    a = cp; b = cq;
    aMin = x; aMax = x + w; bMin = y; bMax = y + h;
  }

  const contentX = (containerW - elW) / 2;
  const contentY = (containerH - elH) / 2;

  // Translate the crop centre to the container centre. With transform-origin at
  // the element (= container) centre C: P → C + scale·R·(P − C), so to send the
  // crop centre Pc to C we need t = −scale·R·(Pc − C).
  const pcx = (a - 0.5) * elW;
  const pcy = (b - 0.5) * elH;
  const rad = (rot * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const tx = -scale * (cos * pcx - sin * pcy);
  const ty = -scale * (sin * pcx + cos * pcy);

  // Clip to the crop rect in the element's LOCAL (un-rotated) space before the
  // transform runs, so off-crop pixels are removed rather than merely shifted.
  const insL = Math.max(0, contentX + aMin * elW);
  const insT = Math.max(0, contentY + bMin * elH);
  const insR = Math.max(0, containerW - (contentX + aMax * elW));
  const insB = Math.max(0, containerH - (contentY + bMax * elH));

  return {
    transform: `translate(${tx}px, ${ty}px) scale(${scale})${rot ? ` rotate(${rot}deg)` : ""}`,
    transformOrigin: "50% 50%",
    clipPath: `inset(${insT}px ${insR}px ${insB}px ${insL}px)`,
    filter,
  };
}

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

  // ── Rotate the photo (and carry the crop with it) ──────────────────────
  /**
   * Rotate by a ±90° (or 180°) delta. The crop corners are stored in the
   * *rotated* frame's 0–1 space, so when the rotation changes they must be
   * re-expressed in the new frame — otherwise the same fractions point at a
   * different region and the selection appears to "slide off" the content
   * (it crops the wrong area). For a 90° multiple the rect stays axis-aligned,
   * so we just map its corners and read off the new x/y/w/h.
   */
  const rotateBy = useCallback((delta: number) => {
    const d = ((delta % 360) + 360) % 360;
    setRotateValue((r) => (((r + delta) % 360) + 360) % 360);
    setCropCorners(({ x, y, w, h }) => {
      // +90° clockwise: corner (cx,cy) → (1−cy, cx)
      if (d === 90) return { x: 1 - y - h, y: x, w: h, h: w };
      // −90° (i.e. +270° CW): corner (cx,cy) → (cy, 1−cx)
      if (d === 270) return { x: y, y: 1 - x - w, w: h, h: w };
      // 180°: corner (cx,cy) → (1−cx, 1−cy)
      if (d === 180) return { x: 1 - x - w, y: 1 - y - h, w, h };
      return { x, y, w, h };
    });
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

    setCropZoomStyle(cropFitStyle(
      { x: cropData.x, y: cropData.y, w: cropData.width, h: cropData.height },
      rot,
      cropData.brightness ?? undefined,
      elW, elH, containerW, containerH,
    ));
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
  /**
   * Compute the **visible content rect** of the photo/video in screen
   * coordinates, taking into account:
   *
   * - `object-contain` letterboxing when the media's aspect ratio differs
   *   from the container's,
   * - the user's current rotation (0 / 90 / 180 / 270°), which swaps the
   *   effective aspect ratio for 90° and 270°,
   * - the edit-mode padding scale ({@link EDIT_CROP_PADDING_SCALE}) that
   *   shrinks the photo away from the container edges so the crop corner
   *   handles are always grabbable.
   *
   * The returned rect describes the area of the screen where the photo is
   * actually drawn — corner-drag pointer coordinates are normalized
   * against this rect, and `CropOverlay` is positioned with it.  Without
   * this correction, pointer coords were normalized against the IMG
   * element's full box (including letterbox padding and the rotated outer
   * bounding box), which produced misaligned crops — especially when
   * combined with rotation.  Returns `null` if the element isn't ready.
   */
  const getMediaRect = useCallback((mediaType: MediaType): DOMRect | null => {
    const el = mediaType === "video"
      ? (videoRef.current as HTMLElement | null)
      : (cropImageRef.current as HTMLElement | null);
    const container = cropContainerRef.current;
    if (!el || !container) return null;

    const containerRect = container.getBoundingClientRect();
    const cw = container.clientWidth;
    const ch = container.clientHeight;

    let nw = 0, nh = 0;
    const v = videoRef.current;
    if (mediaType === "video" && v && v.videoWidth > 0 && v.videoHeight > 0) {
      nw = v.videoWidth; nh = v.videoHeight;
    } else if (cropImageRef.current && cropImageRef.current.naturalWidth > 0) {
      nw = cropImageRef.current.naturalWidth;
      nh = cropImageRef.current.naturalHeight;
    }

    // No natural dims yet — fall back to the element's own bounding rect.
    // This branch is rare (image not yet decoded) and the user can't drag
    // before the image is visible anyway.
    if (nw === 0 || nh === 0 || cw === 0 || ch === 0) {
      return el.getBoundingClientRect();
    }

    const rot = ((rotateValue % 360) + 360) % 360;
    const isSwapped = rot === 90 || rot === 270;
    // Effective aspect ratio AFTER the user's rotation has been applied.
    const aw = isSwapped ? nh : nw;
    const ah = isSwapped ? nw : nh;
    const aspect = aw / ah;

    let dispW: number, dispH: number;
    if (aspect > cw / ch) {
      dispW = cw; dispH = cw / aspect;
    } else {
      dispH = ch; dispW = ch * aspect;
    }

    // Apply the edit-mode padding inset only when the crop tab is active.
    // Other edit tabs (brightness, rotate, trim) keep the photo at full
    // size so the user can preview their changes without an artificial
    // gutter.
    if (editMode && editTab === "crop") {
      dispW *= EDIT_CROP_PADDING_SCALE;
      dispH *= EDIT_CROP_PADDING_SCALE;
    }

    const left = containerRect.left + (cw - dispW) / 2;
    const top = containerRect.top + (ch - dispH) / 2;
    // Synthesise a DOMRect-compatible object — only `left/top/width/height`
    // are read by callers, but we provide all fields for safety.
    return {
      x: left, y: top, left, top, width: dispW, height: dispH,
      right: left + dispW, bottom: top + dispH,
      toJSON() { return this; },
    } as DOMRect;
  }, [rotateValue, editMode, editTab]);

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
    rotateBy,
    computeRotationScale,
    computeCropZoom,
    enterEditMode,
    getMediaRect,
    handleCornerPointerDown, handleCornerPointerMove, handleCornerPointerUp,
  };
}
