import { useEffect, useRef, useState, useCallback } from "react";
import { useParams, useNavigate, useLocation } from "react-router-dom";
import { api } from "../api/client";
import { decrypt } from "../crypto/crypto";
import { useAuthStore } from "../store/auth";
import { db, type MediaType } from "../db";
import AppIcon from "../components/AppIcon";
import { base64ToUint8Array } from "../utils/media";
import PhotoInfoPanel from "../components/viewer/PhotoInfoPanel";
import ViewerEditPanel from "../components/viewer/ViewerEditPanel";
import TagsBar from "../components/viewer/TagsBar";
import LeavePrompt from "../components/viewer/LeavePrompt";
import useZoomPan from "../hooks/useZoomPan";
import usePhotoPreload from "../hooks/usePhotoPreload";

// ── Payload shape (encrypted mode) ───────────────────────────────────────────
interface MediaPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type?: MediaType;
  width: number;
  height: number;
  duration?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64-encoded raw file bytes
}

// ── Navigation context passed via location.state ─────────────────────────────
interface ViewerLocationState {
  /** Array of photo IDs in display order (for prev/next navigation) */
  photoIds?: string[];
  /** Current index within the photoIds array */
  currentIndex?: number;
  /** When viewing from an album, the album ID (enables "Remove" instead of "Delete") */
  albumId?: string;
}

// ── Viewer ────────────────────────────────────────────────────────────────────

export default function Viewer() {
  const { id } = useParams<{ id: string }>();
  const location = useLocation();
  const isPlainMode = location.pathname.startsWith("/photo/plain/");
  const navigate = useNavigate();

  // Destructure navigation context from location.state (passed by Gallery)
  const navState = (location.state ?? {}) as ViewerLocationState;
  const photoIds = navState.photoIds;
  const currentIndex = navState.currentIndex ?? 0;
  const albumId = navState.albumId;
  const hasPrev = !!photoIds && currentIndex > 0;
  const hasNext = !!photoIds && currentIndex < photoIds.length - 1;

  const [mediaUrl, setMediaUrl] = useState<string | null>(null);
  const [filename, setFilename] = useState("");
  const [mimeType, setMimeType] = useState("image/jpeg");
  const [mediaType, setMediaType] = useState<MediaType>("photo");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [videoError, setVideoError] = useState(false);

  // For live preview: show the cached thumbnail while the full media is loading
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);

  // ── Tag state ─────────────────────────────────────────────────────────────
  const [tags, setTags] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState("");
  const [showTagInput, setShowTagInput] = useState(false);
  const [allTags, setAllTags] = useState<string[]>([]);
  const tagInputRef = useRef<HTMLInputElement>(null);

  // ── Favorite state ────────────────────────────────────────────────────────
  const [isFavorite, setIsFavorite] = useState(false);

  // ── Info panel state ───────────────────────────────────────────────────────
  const [showInfoPanel, setShowInfoPanel] = useState(false);
  const [photoInfo, setPhotoInfo] = useState<{
    filename: string;
    mimeType: string;
    width?: number;
    height?: number;
    takenAt?: string | null;
    sizeBytes?: number;
    latitude?: number | null;
    longitude?: number | null;
    createdAt?: string;
    durationSecs?: number | null;
    cameraModel?: string | null;
    albumNames?: string[];
  } | null>(null);

  // ── Slide animation direction ─────────────────────────────────────────────
  const [slideDirection, setSlideDirection] = useState<"left" | "right" | null>(null);
  const [slideKey, setSlideKey] = useState(0);

  // ── Edit mode state ────────────────────────────────────────────────────
  const [editMode, setEditMode] = useState(false);
  const [editTab, setEditTab] = useState<"crop" | "brightness">("crop");
  const [cropData, setCropData] = useState<{ x: number; y: number; width: number; height: number; rotate: number; brightness?: number } | null>(null);
  // Corner-based crop: percentages (0-1) relative to the image
  const [cropCorners, setCropCorners] = useState<{ x: number; y: number; w: number; h: number }>({ x: 0, y: 0, w: 1, h: 1 });
  const [draggingCorner, setDraggingCorner] = useState<string | null>(null);
  const [brightness, setBrightness] = useState(0); // -100 to 100, 0 = no change
  const [showLeavePrompt, setShowLeavePrompt] = useState(false);
  const cropImageRef = useRef<HTMLImageElement>(null);
  const cropContainerRef = useRef<HTMLDivElement>(null);

  const videoRef = useRef<HTMLVideoElement>(null);

  // ── Zoom state (double-tap / pinch-to-zoom) ─────────────────────────────
  const [zoomScale, setZoomScale] = useState(1);
  const [zoomOrigin, setZoomOrigin] = useState<{ x: number; y: number }>({ x: 50, y: 50 });
  const [panOffset, setPanOffset] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const lastTapTime = useRef(0);
  const pinchStartDist = useRef<number | null>(null);
  const pinchStartScale = useRef(1);
  const panStart = useRef<{ x: number; y: number; ox: number; oy: number } | null>(null);

  // Reset zoom when photo changes or entering edit mode
  useEffect(() => {
    setZoomScale(1);
    setPanOffset({ x: 0, y: 0 });
  }, [id, editMode]);

  function handleDoubleClickZoom(e: React.MouseEvent) {
    if (editMode) return;
    const rect = viewerContainerRef.current?.getBoundingClientRect();
    if (!rect) return;
    if (zoomScale > 1) {
      setZoomScale(1);
      setPanOffset({ x: 0, y: 0 });
    } else {
      const x = ((e.clientX - rect.left) / rect.width) * 100;
      const y = ((e.clientY - rect.top) / rect.height) * 100;
      setZoomOrigin({ x, y });
      setZoomScale(2);
      setPanOffset({ x: 0, y: 0 });
    }
  }

  function handleWheel(e: React.WheelEvent) {
    if (editMode) return;
    e.preventDefault();
    const rect = viewerContainerRef.current?.getBoundingClientRect();
    if (!rect) return;
    const x = ((e.clientX - rect.left) / rect.width) * 100;
    const y = ((e.clientY - rect.top) / rect.height) * 100;
    setZoomOrigin({ x, y });
    setZoomScale((prev) => {
      const next = prev - e.deltaY * 0.002;
      if (next <= 1) { setPanOffset({ x: 0, y: 0 }); return 1; }
      return Math.min(next, 5);
    });
  }

  // ── Preload cache for adjacent photos (from hook) ──────────────────────
  const { preloadCache, getCachedPhotoList, preloadAdjacentPhotos } = usePhotoPreload(
    photoIds,
    currentIndex,
    isPlainMode,
  );

  // ── Full-screen overlay state ──────────────────────────────────────────
  const [showOverlay, setShowOverlay] = useState(true);
  const viewerContainerRef = useRef<HTMLDivElement>(null);
  const viewImgRef = useRef<HTMLImageElement>(null);
  const [cropZoomStyle, setCropZoomStyle] = useState<React.CSSProperties>({});

  // Compute crop zoom transform — zooms into the crop region to fill the screen
  const computeCropZoom = useCallback(() => {
    if (!cropData || editMode || !viewImgRef.current || !viewerContainerRef.current) {
      setCropZoomStyle({});
      return;
    }
    const img = viewImgRef.current;
    const container = viewerContainerRef.current;
    const imgW = img.clientWidth;
    const imgH = img.clientHeight;
    const containerW = container.clientWidth;
    const containerH = container.clientHeight;

    if (imgW === 0 || imgH === 0 || containerW === 0 || containerH === 0) return;

    const cropPixW = cropData.width * imgW;
    const cropPixH = cropData.height * imgH;
    const scaleW = containerW / cropPixW;
    const scaleH = containerH / cropPixH;
    const scale = Math.min(scaleW, scaleH);

    const cx = cropData.x + cropData.width / 2;
    const cy = cropData.y + cropData.height / 2;

    setCropZoomStyle({
      transform: `translate(${(0.5 - cx) * 100}%, ${(0.5 - cy) * 100}%) scale(${scale})`,
      transformOrigin: `${cx * 100}% ${cy * 100}%`,
      filter: cropData.brightness ? `brightness(${1 + (cropData.brightness ?? 0) / 100})` : undefined,
    });
  }, [cropData, editMode]);

  // Recompute crop zoom on resize
  useEffect(() => {
    computeCropZoom();
    window.addEventListener('resize', computeCropZoom);
    return () => window.removeEventListener('resize', computeCropZoom);
  }, [computeCropZoom]);

  // ── Load tags + favorite state for current photo ─────────────────────────
  useEffect(() => {
    if (!id) return;
    setTags([]);
    setIsFavorite(false);
    if (isPlainMode) {
      api.tags.getPhotoTags(id).then((res) => setTags(res.tags)).catch(() => {});
      api.tags.list().then((res) => setAllTags(res.tags)).catch(() => {});
      // Load favorite and crop from photo metadata
      getCachedPhotoList().then((photos) => {
        const photo = photos.find((p) => p.id === id);
        if (photo) {
          setIsFavorite(!!photo.is_favorite);
          setPhotoInfo({
            filename: photo.filename,
            mimeType: photo.mime_type,
            width: photo.width,
            height: photo.height,
            takenAt: photo.taken_at,
            sizeBytes: photo.size_bytes,
            latitude: photo.latitude,
            longitude: photo.longitude,
            createdAt: photo.created_at,
            durationSecs: photo.duration_secs,
            cameraModel: photo.camera_model,
          });
          if (photo.crop_metadata) {
            try { setCropData(JSON.parse(photo.crop_metadata)); } catch { setCropData(null); }
          } else {
            setCropData(null);
          }
        }
      }).catch(() => {});
    } else {
      // Encrypted mode: load crop data and metadata from local IndexedDB
      db.photos.get(id).then(async (cached) => {
        if (cached) {
          // Look up album names for this photo
          const allAlbums = await db.albums.toArray();
          const albumNames = allAlbums
            .filter((a) => a.photoBlobIds.includes(id!))
            .map((a) => a.name);
          setPhotoInfo({
            filename: cached.filename,
            mimeType: cached.mimeType,
            width: cached.width,
            height: cached.height,
            takenAt: cached.takenAt ? new Date(cached.takenAt).toISOString() : null,
            latitude: cached.latitude,
            longitude: cached.longitude,
            albumNames,
          });
          if (cached.cropData) {
            try { setCropData(JSON.parse(cached.cropData)); } catch { setCropData(null); }
          } else {
            setCropData(null);
          }
        } else {
          setCropData(null);
        }
      }).catch(() => { setCropData(null); });
    }
  }, [id, isPlainMode]);

  // Auto-focus tag input when shown
  useEffect(() => {
    if (showTagInput) tagInputRef.current?.focus();
  }, [showTagInput]);

  async function handleAddTag() {
    const tag = tagInput.trim().toLowerCase();
    if (!tag || !id) return;
    try {
      await api.tags.add(id, tag);
      setTags((prev) => (prev.includes(tag) ? prev : [...prev, tag].sort()));
      if (!allTags.includes(tag)) setAllTags((prev) => [...prev, tag].sort());
      setTagInput("");
    } catch {
      // ignore
    }
  }

  async function handleRemoveTag(tag: string) {
    if (!id) return;
    try {
      await api.tags.remove(id, tag);
      setTags((prev) => prev.filter((t) => t !== tag));
    } catch {
      // ignore
    }
  }

  async function handleToggleFavorite() {
    if (!id || !isPlainMode) return;
    try {
      const res = await api.photos.toggleFavorite(id);
      setIsFavorite(res.is_favorite);
    } catch {
      // ignore
    }
  }

  // Initialize crop corners and brightness from existing metadata when entering edit mode
  function enterEditMode() {
    if (cropData) {
      setCropCorners({ x: cropData.x, y: cropData.y, w: cropData.width, h: cropData.height });
      setBrightness(cropData.brightness ?? 0);
    } else {
      setCropCorners({ x: 0, y: 0, w: 1, h: 1 });
      setBrightness(0);
    }
    setEditTab("brightness");
    setEditMode(true);
  }

  async function handleSaveEdit() {
    if (!id) return;
    const c = cropCorners;
    const isDefaultCrop = c.x <= 0.01 && c.y <= 0.01 && c.w >= 0.99 && c.h >= 0.99;
    const isDefaultBrightness = Math.abs(brightness) < 1;
    // If everything is default, clear metadata
    if (isDefaultCrop && isDefaultBrightness) {
      try {
        if (isPlainMode) {
          await api.photos.setCrop(id, null);
        } else {
          await db.photos.update(id, { cropData: undefined });
        }
        setCropData(null);
      } catch { /* ignore */ }
    } else {
      const meta = {
        x: Math.max(0, Math.min(1, c.x)),
        y: Math.max(0, Math.min(1, c.y)),
        width: Math.max(0.05, Math.min(1, c.w)),
        height: Math.max(0.05, Math.min(1, c.h)),
        rotate: 0,
        brightness,
      };
      try {
        if (isPlainMode) {
          await api.photos.setCrop(id, JSON.stringify(meta));
        } else {
          await db.photos.update(id, { cropData: JSON.stringify(meta) });
        }
        setCropData(meta);
      } catch { /* ignore */ }
    }
    setEditMode(false);
  }

  async function handleClearCrop() {
    if (!id) return;
    try {
      if (isPlainMode) {
        await api.photos.setCrop(id, null);
      } else {
        await db.photos.update(id, { cropData: undefined });
      }
      setCropData(null);
      setCropCorners({ x: 0, y: 0, w: 1, h: 1 });
      setBrightness(0);
    } catch { /* ignore */ }
  }

  // ── Leave-photo handlers (save/discard prompt in edit mode) ──────────
  async function handleLeaveAndSave() {
    await handleSaveEdit();
    setShowLeavePrompt(false);
    navigate("/gallery");
  }

  function handleLeaveAndDiscard() {
    setEditMode(false);
    setShowLeavePrompt(false);
    navigate("/gallery");
  }

  // ── Corner drag handlers ────────────────────────────────────────────────
  function getImageRect() {
    return cropImageRef.current?.getBoundingClientRect() ?? null;
  }

  function handleCornerPointerDown(corner: string) {
    return (e: React.PointerEvent) => {
      e.preventDefault();
      e.stopPropagation();
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
      setDraggingCorner(corner);
    };
  }

  function handleCornerPointerMove(e: React.PointerEvent) {
    if (!draggingCorner) return;
    const imgRect = getImageRect();
    if (!imgRect) return;
    const px = Math.max(0, Math.min(1, (e.clientX - imgRect.left) / imgRect.width));
    const py = Math.max(0, Math.min(1, (e.clientY - imgRect.top) / imgRect.height));
    setCropCorners((prev) => {
      const minSize = 0.05;
      let { x, y, w, h } = prev;
      if (draggingCorner === "tl") {
        const newX = Math.min(px, x + w - minSize);
        const newY = Math.min(py, y + h - minSize);
        w = w + (x - newX); h = h + (y - newY);
        x = newX; y = newY;
      } else if (draggingCorner === "tr") {
        const newR = Math.max(px, x + minSize);
        const newY = Math.min(py, y + h - minSize);
        w = newR - x; h = h + (y - newY); y = newY;
      } else if (draggingCorner === "bl") {
        const newX = Math.min(px, x + w - minSize);
        const newB = Math.max(py, y + minSize);
        w = w + (x - newX); x = newX; h = newB - y;
      } else if (draggingCorner === "br") {
        const newR = Math.max(px, x + minSize);
        const newB = Math.max(py, y + minSize);
        w = newR - x; h = newB - y;
      }
      return { x: Math.max(0, x), y: Math.max(0, y), w: Math.min(w, 1 - x), h: Math.min(h, 1 - y) };
    });
  }

  function handleCornerPointerUp() {
    setDraggingCorner(null);
  }

  // Filter suggestions: tags the user has used before that aren't on this photo
  const tagSuggestions = allTags.filter(
    (t) => !tags.includes(t) && t.includes(tagInput.toLowerCase())
  ).slice(0, 5);

  const navigateToPhoto = useCallback(
    (index: number) => {
      if (!photoIds || index < 0 || index >= photoIds.length) return;
      const nextId = photoIds[index];
      const prefix = isPlainMode ? "/photo/plain/" : "/photo/";
      // Determine slide direction: going forward = slide from right, going back = slide from left
      setSlideDirection(index > currentIndex ? "right" : "left");
      setSlideKey((k) => k + 1);
      navigate(`${prefix}${nextId}`, {
        replace: true,
        state: { photoIds, currentIndex: index } satisfies ViewerLocationState,
      });
    },
    [photoIds, isPlainMode, navigate, currentIndex]
  );

  const goPrev = useCallback(() => {
    if (hasPrev) navigateToPhoto(currentIndex - 1);
  }, [hasPrev, currentIndex, navigateToPhoto]);

  const goNext = useCallback(() => {
    if (hasNext) navigateToPhoto(currentIndex + 1);
  }, [hasNext, currentIndex, navigateToPhoto]);

  // ── Swipe / pinch-to-zoom / double-tap handling ─────────────────────────
  const touchStartX = useRef<number | null>(null);
  const touchStartY = useRef<number | null>(null);
  const swiped = useRef(false);

  function handleTouchStart(e: React.TouchEvent) {
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
      swiped.current = true; // prevent swipe when pinching
      return;
    }
    touchStartX.current = e.touches[0].clientX;
    touchStartY.current = e.touches[0].clientY;
    swiped.current = false;

    // Double-tap detection
    const now = Date.now();
    if (now - lastTapTime.current < 300 && !editMode) {
      // Double tap — toggle zoom
      const rect = viewerContainerRef.current?.getBoundingClientRect();
      if (rect) {
        if (zoomScale > 1) {
          setZoomScale(1);
          setPanOffset({ x: 0, y: 0 });
        } else {
          const x = ((e.touches[0].clientX - rect.left) / rect.width) * 100;
          const y = ((e.touches[0].clientY - rect.top) / rect.height) * 100;
          setZoomOrigin({ x, y });
          setZoomScale(2);
          setPanOffset({ x: 0, y: 0 });
        }
      }
      swiped.current = true; // prevent swipe
      lastTapTime.current = 0;
      return;
    }
    lastTapTime.current = now;

    // Pan start when zoomed in
    if (zoomScale > 1) {
      panStart.current = { x: e.touches[0].clientX, y: e.touches[0].clientY, ox: panOffset.x, oy: panOffset.y };
    }
  }

  function handleTouchMove(e: React.TouchEvent) {
    if (editMode) return;
    // Pinch-to-zoom
    if (e.touches.length === 2 && pinchStartDist.current !== null) {
      const dx = e.touches[0].clientX - e.touches[1].clientX;
      const dy = e.touches[0].clientY - e.touches[1].clientY;
      const dist = Math.sqrt(dx * dx + dy * dy);
      const ratio = dist / pinchStartDist.current;
      const newScale = Math.max(1, Math.min(5, pinchStartScale.current * ratio));
      setZoomScale(newScale);
      if (newScale <= 1) setPanOffset({ x: 0, y: 0 });
      return;
    }
    // Pan when zoomed
    if (zoomScale > 1 && panStart.current && e.touches.length === 1) {
      const dx = e.touches[0].clientX - panStart.current.x;
      const dy = e.touches[0].clientY - panStart.current.y;
      setPanOffset({ x: panStart.current.ox + dx, y: panStart.current.oy + dy });
      swiped.current = true; // prevent swipe navigation when panning
    }
  }

  function handleTouchEnd(e: React.TouchEvent) {
    if (editMode) return;
    const wasPinching = pinchStartDist.current !== null;
    pinchStartDist.current = null;
    panStart.current = null;

    // Snap back to normal mode when zoom reaches 1× (after pinch-out)
    if (zoomScale <= 1.05) {
      setZoomScale(1);
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
        // Swipe up → show info panel
        setShowInfoPanel(true);
      } else {
        // Swipe down → close viewer (or dismiss info panel)
        if (showInfoPanel) {
          setShowInfoPanel(false);
        } else {
          navigate(-1);
        }
      }
    }
    touchStartX.current = null;
    touchStartY.current = null;
  }

  // ── Load media on id change (with preload cache) ───────────────────────
  useEffect(() => {
    if (!id) return;

    // Check preload cache first — if we have a hit, display instantly
    const cached = preloadCache.current.get(id);
    if (cached) {
      // Don't revoke the old mediaUrl if it's still in the preload cache
      setMediaUrl((prev) => {
        if (prev && !Array.from(preloadCache.current.values()).some(e => e.url === prev)) {
          URL.revokeObjectURL(prev);
        }
        return cached.url;
      });
      setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
      setFilename(cached.filename);
      setMimeType(cached.mimeType);
      setMediaType(cached.mediaType);
      setIsFavorite(cached.isFavorite);
      if (cached.cropData) {
        setCropData(cached.cropData);
      } else {
        setCropData(null);
      }
      setLoading(false);
      setError("");
      setVideoError(false);
    } else {
      // Cache miss — load normally
      setMediaUrl((prev) => {
        if (prev && !Array.from(preloadCache.current.values()).some(e => e.url === prev)) {
          URL.revokeObjectURL(prev);
        }
        return null;
      });
      setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
      setLoading(true);
      setError("");
      setVideoError(false);

      if (isPlainMode) {
        loadPlainMedia(id);
      } else {
        // Show cached thumbnail immediately for a live-preview feel
        db.photos.get(id).then((dbCached) => {
          if (dbCached?.thumbnailData) {
            const url = URL.createObjectURL(new Blob([dbCached.thumbnailData], { type: "image/jpeg" }));
            setPreviewUrl(url);
          }
        });
        loadEncryptedMedia(id);
      }
    }

    // Preload adjacent photos (±5 direction-aware) after a very short delay
    const preloadTimer = setTimeout(() => {
      preloadAdjacentPhotos(id);
    }, 50);

    return () => clearTimeout(preloadTimer);
  }, [id]);

  // ── Keyboard navigation ────────────────────────────────────────────────
  useEffect(() => {
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (showLeavePrompt) {
          setShowLeavePrompt(false);
        } else if (editMode) {
          setShowLeavePrompt(true);
        } else {
          navigate("/gallery");
        }
        return;
      }
      if (editMode) return; // Block photo navigation in edit mode
      if (e.key === "ArrowLeft") goPrev();
      if (e.key === "ArrowRight") goNext();
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [goPrev, goNext, navigate, editMode, showLeavePrompt]);

  /** Load a plain-mode photo — check IndexedDB cache first, then fetch */
  async function loadPlainMedia(photoId: string) {
    setLoading(true);
    setError("");
    try {
      // Fetch photo metadata to get filename and media type (uses cached list)
      const photos = await getCachedPhotoList();
      const photo = photos.find((p) => p.id === photoId);
      let resolvedFilename = "";
      let resolvedMime = "image/jpeg";
      let resolved: MediaType = "photo";
      let photoCropData = null;
      let photoIsFavorite = false;
      if (photo) {
        resolvedFilename = photo.filename;
        resolvedMime = photo.mime_type;
        resolved =
          photo.media_type === "gif" ? "gif"
          : photo.media_type === "video" ? "video"
          : "photo";
        photoIsFavorite = !!photo.is_favorite;
        if (photo.crop_metadata) {
          try { photoCropData = JSON.parse(photo.crop_metadata); } catch { /* ignore */ }
        }
        setFilename(resolvedFilename);
        setMimeType(resolvedMime);
        setMediaType(resolved);
      }

      // Check IndexedDB full-photo cache for instant display
      const idbCached = await db.fullPhotos?.get(photoId);
      if (idbCached?.data) {
        const blob = new Blob([idbCached.data], { type: idbCached.mimeType });
        const url = URL.createObjectURL(blob);
        setMediaUrl(url);
        preloadCache.current.set(photoId, {
          url, filename: resolvedFilename, mimeType: resolvedMime,
          mediaType: resolved, cropData: photoCropData, isFavorite: photoIsFavorite,
        });
        setLoading(false);
        return;
      }

      // Cache miss — fetch from server (use /web endpoint for browser-compatible format)
      const { accessToken } = useAuthStore.getState();
      const headers: Record<string, string> = { "X-Requested-With": "SimplePhotos" };
      if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
      const fileRes = await fetch(api.photos.webUrl(photoId), { headers });
      if (!fileRes.ok) throw new Error(`Failed to load photo: ${fileRes.status}`);
      const blob = await fileRes.blob();
      const url = URL.createObjectURL(blob);
      setMediaUrl(url);

      // Store in preload cache so swiping back is instant
      preloadCache.current.set(photoId, {
        url,
        filename: resolvedFilename,
        mimeType: resolvedMime,
        mediaType: resolved,
        cropData: photoCropData,
        isFavorite: photoIsFavorite,
      });

      // Also cache in IndexedDB for cross-session persistence
      if (blob.size < 50 * 1024 * 1024) {
        try {
          const arrayBuf = await blob.arrayBuffer();
          await db.fullPhotos?.put({
            photoId, filename: resolvedFilename, mimeType: resolvedMime,
            mediaType: resolved, cropData: photo?.crop_metadata ?? undefined,
            isFavorite: photoIsFavorite, data: arrayBuf, cachedAt: Date.now(),
          });
        } catch { /* non-fatal */ }
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load media");
    } finally {
      setLoading(false);
    }
  }

  /** Load an encrypted blob — check IndexedDB cache first, then decrypt */
  async function loadEncryptedMedia(blobId: string) {
    setLoading(true);
    setError("");
    try {
      // Check IndexedDB full-photo cache for instant display
      const idbCached = await db.fullPhotos?.get(blobId);
      if (idbCached?.data) {
        const blob = new Blob([idbCached.data], { type: idbCached.mimeType });
        const url = URL.createObjectURL(blob);
        setMediaUrl(url);
        setFilename(idbCached.filename);
        setMimeType(idbCached.mimeType);
        const resolvedType: MediaType =
          idbCached.mediaType === "gif" ? "gif"
          : idbCached.mediaType === "video" ? "video"
          : "photo";
        setMediaType(resolvedType);

        let photoCropData = null;
        if (idbCached.cropData) {
          try { photoCropData = JSON.parse(idbCached.cropData); } catch { /* ignore */ }
        }
        preloadCache.current.set(blobId, {
          url, filename: idbCached.filename, mimeType: idbCached.mimeType,
          mediaType: resolvedType, cropData: photoCropData,
          isFavorite: idbCached.isFavorite ?? false,
        });
        if (previewUrl) {
          URL.revokeObjectURL(previewUrl);
          setPreviewUrl(null);
        }
        setLoading(false);
        return;
      }

      // Cache miss — download, decrypt, display
      const encrypted = await api.blobs.download(blobId);
      const decrypted = await decrypt(encrypted);
      const payload: MediaPayload = JSON.parse(new TextDecoder().decode(decrypted));

      setFilename(payload.filename);
      setMimeType(payload.mime_type);

      // Derive media type from payload, then MIME, then default to photo
      const resolvedType: MediaType =
        payload.media_type ??
        (payload.mime_type === "image/gif"
          ? "gif"
          : payload.mime_type.startsWith("video/")
          ? "video"
          : payload.mime_type.startsWith("audio/")
          ? "audio"
          : "photo");
      setMediaType(resolvedType);

      // Decode base64 → Blob → Object URL
      const bytes = base64ToUint8Array(payload.data).buffer as ArrayBuffer;
      const blob = new Blob([bytes], { type: payload.mime_type });
      const url = URL.createObjectURL(blob);
      setMediaUrl(url);

      // Load crop data from IndexedDB for cache entry
      let photoCropData = null;
      const dbEntry = await db.photos.get(blobId);
      if (dbEntry?.cropData) {
        try { photoCropData = JSON.parse(dbEntry.cropData); } catch { /* ignore */ }
      }

      // Store in preload cache so swiping back is instant
      preloadCache.current.set(blobId, {
        url,
        filename: payload.filename,
        mimeType: payload.mime_type,
        mediaType: resolvedType,
        cropData: photoCropData,
        isFavorite: false,
      });

      // Cache decrypted data in IndexedDB for cross-session persistence
      if (blob.size < 50 * 1024 * 1024) {
        try {
          await db.fullPhotos?.put({
            photoId: blobId, filename: payload.filename, mimeType: payload.mime_type,
            mediaType: resolvedType, cropData: dbEntry?.cropData ?? undefined,
            isFavorite: false, data: bytes, cachedAt: Date.now(),
          });
        } catch { /* non-fatal */ }
      }

      // Revoke the preview now that full media is ready
      if (previewUrl) {
        URL.revokeObjectURL(previewUrl);
        setPreviewUrl(null);
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load media");
    } finally {
      setLoading(false);
    }
  }

  async function handleDelete() {
    const msg = "Move this item to trash? You can restore it within 30 days.";
    if (!id || !confirm(msg)) return;
    try {
      if (isPlainMode) {
        // Server soft-deletes to trash for plain mode
        await api.photos.delete(id);
      } else {
        // Encrypted mode: soft-delete blob to trash with client metadata
        const cached = await db.photos.get(id);
        const result = await api.blobs.softDelete(id, {
          thumbnail_blob_id: cached?.thumbnailBlobId,
          filename: cached?.filename ?? "unknown",
          mime_type: cached?.mimeType ?? "application/octet-stream",
          media_type: cached?.mediaType,
          size_bytes: 0,
          width: cached?.width,
          height: cached?.height,
          duration_secs: cached?.duration,
          taken_at: cached?.takenAt
            ? new Date(cached.takenAt).toISOString()
            : undefined,
        });
        // Move to local trash table so we can show thumbnails in Trash view
        if (cached) {
          await db.trash.put({
            trashId: result.trash_id,
            blobId: id,
            thumbnailBlobId: cached.thumbnailBlobId,
            filename: cached.filename,
            mimeType: cached.mimeType,
            mediaType: cached.mediaType,
            width: cached.width,
            height: cached.height,
            takenAt: cached.takenAt,
            deletedAt: Date.now(),
            expiresAt: result.expires_at,
            thumbnailData: cached.thumbnailData,
            duration: cached.duration,
            albumIds: cached.albumIds ?? [],
          });
        }
        await db.photos.delete(id);
      }
      navigate("/gallery");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Delete failed");
    }
  }

  async function handleRemoveFromAlbum() {
    if (!id || !albumId) return;
    try {
      const album = await db.albums.get(albumId);
      if (!album) return;
      const updated = album.photoBlobIds.filter((bid: string) => bid !== id);

      // Delete old manifest blob
      if (album.manifestBlobId) {
        try { await api.blobs.delete(album.manifestBlobId); } catch { /* ok */ }
      }

      // Upload new manifest
      const payload = JSON.stringify({
        v: 1,
        album_id: album.albumId,
        name: album.name,
        created_at: new Date(album.createdAt).toISOString(),
        cover_photo_blob_id: album.coverPhotoBlobId || null,
        photo_blob_ids: updated,
      });
      const { encrypt: enc, sha256Hex: sha } = await import("../crypto/crypto");
      const encrypted = await enc(new TextEncoder().encode(payload));
      const hash = await sha(new Uint8Array(encrypted));
      const res = await api.blobs.upload(encrypted, "album_manifest", hash);

      await db.albums.put({ ...album, photoBlobIds: updated, manifestBlobId: res.blob_id });

      // Navigate to next photo or back to album
      if (photoIds && photoIds.length > 1) {
        const remaining = photoIds.filter((pid) => pid !== id);
        const nextIdx = Math.min(currentIndex, remaining.length - 1);
        const nextId = remaining[nextIdx];
        const prefix = isPlainMode ? "/photo/plain/" : "/photo/";
        navigate(prefix + nextId, { replace: true, state: { photoIds: remaining, currentIndex: nextIdx, albumId } });
      } else {
        navigate(`/album/${albumId}`);
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Remove failed");
    }
  }

  function handleDownload() {
    if (!mediaUrl) return;
    const a = document.createElement("a");
    a.href = mediaUrl;
    a.download = filename || "media";
    a.click();
  }

  // ── Render ────────────────────────────────────────────────────────────────────
  return (
    <div
      className="fixed inset-0 bg-black select-none"
      onTouchStart={handleTouchStart}
      onTouchMove={handleTouchMove}
      onTouchEnd={handleTouchEnd}
    >
      {/* Top bar (overlay) */}
      <div className={`absolute top-0 left-0 right-0 z-30 transition-opacity duration-300 ${
        showOverlay || editMode ? 'opacity-100' : 'opacity-0 pointer-events-none'
      }`}>
      <div className="flex items-center justify-between px-4 py-3 bg-black/80">
        <button
          onClick={() => {
            if (editMode) {
              setShowLeavePrompt(true);
            } else {
              navigate("/gallery");
            }
          }}
          className="text-white hover:text-gray-300 flex items-center justify-center w-8 h-8 rounded-full hover:bg-white/20 transition-colors"
          title="Back"
        >
          <AppIcon name="back-arrow" size="w-5 h-5" themed={false} className="invert" />
        </button>
        <div className="flex gap-3 items-center">
          {/* Info button — shows metadata panel */}
          <button
            onClick={() => setShowInfoPanel((v) => !v)}
            className={`flex items-center justify-center w-8 h-8 rounded-full transition-colors ${
              showInfoPanel ? "bg-blue-600 text-white" : "text-white hover:bg-white/20"
            }`}
            title="Info"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          </button>
          {/* Edit button — available for all photos (not videos/gifs) */}
          {mediaType === "photo" && (
            <button
              onClick={() => { if (editMode) { setEditMode(false); } else { enterEditMode(); } }}
              className={`flex items-center gap-1 px-2 py-1 rounded text-sm font-medium transition-colors ${
                editMode
                  ? "bg-blue-600 text-white"
                  : "text-white hover:bg-white/20"
              }`}
              title="Edit"
            >
              Edit
            </button>
          )}
          {/* Favorite button — plain mode only */}
          {isPlainMode && (
            <button
              onClick={handleToggleFavorite}
              className={`hover:scale-110 transition-transform ${isFavorite ? "text-yellow-400" : "text-white hover:text-yellow-300"}`}
              title={isFavorite ? "Unfavorite" : "Favorite"}
            >
              {isFavorite ? (
                <svg className="w-5 h-5" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
                </svg>
              ) : (
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z" />
                </svg>
              )}
            </button>
          )}
          <button
            onClick={handleDownload}
            className="text-white hover:text-gray-300 flex items-center justify-center w-8 h-8 rounded-full hover:bg-white/20 transition-colors"
            disabled={!mediaUrl}
            title="Download"
          >
            <AppIcon name="download" size="w-5 h-5" themed={false} className="invert" />
          </button>
          {albumId ? (
            <button
              onClick={handleRemoveFromAlbum}
              className="text-orange-400 hover:text-orange-300 flex items-center justify-center w-8 h-8 rounded-full hover:bg-white/20 transition-colors"
              title="Remove from album"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M15 12H9m12 0a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
            </button>
          ) : (
            <button
              onClick={handleDelete}
              className="text-red-400 hover:text-red-300 flex items-center justify-center w-8 h-8 rounded-full hover:bg-white/20 transition-colors"
              title="Delete"
            >
              <AppIcon name="trashcan" size="w-5 h-5" themed={false} className="invert" />
            </button>
          )}
        </div>
      </div>
      </div>{/* end top bar overlay */}

      {/* Content area — fills entire viewport for true full-screen */}
      <div
        ref={viewerContainerRef}
        className="absolute inset-0 flex items-center justify-center overflow-hidden"
        onClick={(e) => {
          if (swiped.current) return;
          if ((e.target as HTMLElement).closest('button')) return;
          if (!editMode) setShowOverlay(prev => !prev);
        }}
        onDoubleClick={handleDoubleClickZoom}
        onWheel={handleWheel}
      >
        {/* Live preview: blurred thumbnail shown while full media loads */}
        {previewUrl && loading && (
          <img
            src={previewUrl}
            alt="preview"
            className="absolute inset-0 w-full h-full object-contain blur-sm opacity-60 pointer-events-none"
          />
        )}

        {loading && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="w-8 h-8 border-2 border-white/30 border-t-white rounded-full animate-spin" />
          </div>
        )}

        {error && (
          <p className="text-red-400 text-sm z-10">{error}</p>
        )}

        {/* ── Previous arrow (left side) ─────────────────────────────── */}
        {hasPrev && !editMode && (
          <button
            onClick={goPrev}
            className="absolute left-2 top-1/2 -translate-y-1/2 z-20 w-10 h-10 md:w-12 md:h-12 flex items-center justify-center rounded-full bg-black/50 hover:bg-black/80 text-white transition-colors"
            aria-label="Previous photo"
          >
            <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 19.5L8.25 12l7.5-7.5" />
            </svg>
          </button>
        )}

        {/* ── Next arrow (right side) ────────────────────────────────── */}
        {hasNext && !editMode && (
          <button
            onClick={goNext}
            className="absolute right-2 top-1/2 -translate-y-1/2 z-20 w-10 h-10 md:w-12 md:h-12 flex items-center justify-center rounded-full bg-black/50 hover:bg-black/80 text-white transition-colors"
            aria-label="Next photo"
          >
            <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M8.25 4.5l7.5 7.5-7.5 7.5" />
            </svg>
          </button>
        )}

        {/* ── Slide animation wrapper ──────────────────────────────────── */}
        <div
          key={slideKey}
          className={`w-full h-full flex items-center justify-center ${
            slideDirection === "right" ? "slide-in-right" :
            slideDirection === "left" ? "slide-in-left" : ""
          }`}
        >
        {/* ── Photo / GIF viewer (normal mode) ───────────────────────────── */}
        {mediaUrl && (mediaType === "photo" || mediaType === "gif") && !editMode && (
          <img
            ref={viewImgRef}
            src={mediaUrl}
            alt={filename}
            className="max-w-full max-h-full object-contain transition-transform duration-150"
            onLoad={computeCropZoom}
            style={{
              imageRendering: mediaType === "gif" ? "auto" : undefined,
              ...(cropData && zoomScale <= 1 ? cropZoomStyle : {}),
              ...(zoomScale > 1 ? {
                transform: `scale(${zoomScale}) translate(${panOffset.x / zoomScale}px, ${panOffset.y / zoomScale}px)`,
                transformOrigin: `${zoomOrigin.x}% ${zoomOrigin.y}%`,
                cursor: 'grab',
              } : {}),
            }}
          />
        )}

        {/* ── Edit mode overlay ───────────────────────────────────────────── */}
        {editMode && mediaUrl && mediaType === "photo" && (
          <div
            ref={cropContainerRef}
            className="relative w-full h-full flex items-center justify-center"
            onPointerMove={editTab === "crop" ? handleCornerPointerMove : undefined}
            onPointerUp={editTab === "crop" ? handleCornerPointerUp : undefined}
          >
            <img
              ref={cropImageRef}
              src={mediaUrl}
              alt={filename}
              className="max-w-full max-h-full object-contain pointer-events-none"
              draggable={false}
              style={{
                filter: brightness !== 0 ? `brightness(${1 + brightness / 100})` : undefined,
              }}
            />
            {/* Crop overlay with 4 corner dots and connecting lines */}
            {editTab === "crop" && cropImageRef.current && (() => {
              const imgRect = cropImageRef.current!.getBoundingClientRect();
              const containerRect = cropContainerRef.current?.getBoundingClientRect();
              if (!containerRect) return null;
              const ox = imgRect.left - containerRect.left;
              const oy = imgRect.top - containerRect.top;
              const iw = imgRect.width;
              const ih = imgRect.height;
              const c = cropCorners;
              const tl = { x: ox + c.x * iw, y: oy + c.y * ih };
              const tr = { x: ox + (c.x + c.w) * iw, y: oy + c.y * ih };
              const bl = { x: ox + c.x * iw, y: oy + (c.y + c.h) * ih };
              const br = { x: ox + (c.x + c.w) * iw, y: oy + (c.y + c.h) * ih };
              const dotSize = 16;
              const corners = [
                { key: "tl", ...tl },
                { key: "tr", ...tr },
                { key: "bl", ...bl },
                { key: "br", ...br },
              ];
              return (
                <>
                  {/* Darkened area outside crop */}
                  <div
                    className="absolute pointer-events-none z-20"
                    style={{
                      left: tl.x, top: tl.y,
                      width: tr.x - tl.x, height: bl.y - tl.y,
                      boxShadow: "0 0 0 9999px rgba(0,0,0,0.5)",
                    }}
                  />
                  {/* White border lines */}
                  <svg className="absolute inset-0 w-full h-full pointer-events-none z-30">
                    <line x1={tl.x} y1={tl.y} x2={tr.x} y2={tr.y} stroke="white" strokeWidth={2} />
                    <line x1={tr.x} y1={tr.y} x2={br.x} y2={br.y} stroke="white" strokeWidth={2} />
                    <line x1={br.x} y1={br.y} x2={bl.x} y2={bl.y} stroke="white" strokeWidth={2} />
                    <line x1={bl.x} y1={bl.y} x2={tl.x} y2={tl.y} stroke="white" strokeWidth={2} />
                  </svg>
                  {/* 4 draggable corner dots */}
                  {corners.map((corner) => (
                    <div
                      key={corner.key}
                      onPointerDown={handleCornerPointerDown(corner.key)}
                      className="absolute z-40 rounded-full bg-white border-2 border-white shadow-lg cursor-grab active:cursor-grabbing"
                      style={{
                        width: dotSize, height: dotSize,
                        left: corner.x - dotSize / 2,
                        top: corner.y - dotSize / 2,
                        touchAction: "none",
                      }}
                    />
                  ))}
                </>
              );
            })()}
          </div>
        )}

        {/* ── Video player ───────────────────────────────────────────────── */}
        {mediaUrl && mediaType === "video" && !videoError && (
          <video
            ref={videoRef}
            src={mediaUrl}
            controls
            playsInline
            autoPlay={false}
            className="max-w-full max-h-full"
            style={{ background: "black" }}
            onError={() => setVideoError(true)}
          />
        )}

        {/* ── Video format not supported fallback ────────────────────────── */}
        {mediaUrl && mediaType === "video" && videoError && (
          <div className="w-full h-full flex flex-col items-center justify-center" style={{ background: "black" }}>
            <svg className="w-16 h-16 text-gray-500 mb-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="m15.75 10.5 4.72-4.72a.75.75 0 0 1 1.28.53v11.38a.75.75 0 0 1-1.28.53l-4.72-4.72M4.5 18.75h9a2.25 2.25 0 0 0 2.25-2.25v-9a2.25 2.25 0 0 0-2.25-2.25h-9A2.25 2.25 0 0 0 2.25 7.5v9a2.25 2.25 0 0 0 2.25 2.25Z" />
            </svg>
            <p className="text-gray-300 text-sm mb-1">This video format is not supported by your browser.</p>
            <p className="text-gray-500 text-xs mb-4 px-4 text-center truncate max-w-[80%]">{filename}</p>
            <button
              onClick={handleDownload}
              className="px-4 py-2 bg-blue-600 text-white text-sm rounded-lg hover:bg-blue-700 transition-colors"
            >
              Download Original
            </button>
          </div>
        )}

        {/* ── Audio player (black screen + playbar) ──────────────────────── */}
        {mediaUrl && mediaType === "audio" && (
          <div className="w-full h-full flex flex-col items-center justify-center" style={{ background: "black" }}>
            <div className="text-gray-400 text-6xl mb-6">♫</div>
            <p className="text-gray-300 text-sm mb-6 px-4 text-center truncate max-w-[80%]">{filename}</p>
            <audio
              src={mediaUrl}
              controls
              autoPlay={false}
              className="w-full max-w-md"
              style={{ filter: "invert(1) hue-rotate(180deg)", opacity: 0.85 }}
            />
          </div>
        )}
        </div>{/* end slide animation wrapper */}
      </div>

      {/* ── Edit panel (Crop / Brightness tabs + Save) ───────────────── */}
      {editMode && mediaUrl && mediaType === "photo" && (
        <ViewerEditPanel
          editTab={editTab}
          setEditTab={setEditTab}
          brightness={brightness}
          setBrightness={setBrightness}
          cropData={cropData}
          onSave={handleSaveEdit}
          onClear={handleClearCrop}
          onCancel={() => setEditMode(false)}
        />
      )}

      {/* Bottom meta bar (shown when media is loaded, overlay) */}
      {mediaUrl && !editMode && (
        <TagsBar
          show={showOverlay}
          isPlainMode={isPlainMode}
          tags={tags}
          showTagInput={showTagInput}
          tagInput={tagInput}
          setTagInput={setTagInput}
          setShowTagInput={setShowTagInput}
          tagSuggestions={tagSuggestions}
          onAddTag={handleAddTag}
          onRemoveTag={handleRemoveTag}
          tagInputRef={tagInputRef}
        />
      )}

      {/* ── Info Panel (slides up from bottom) ───────────────────── */}
      <PhotoInfoPanel
        show={showInfoPanel}
        onClose={() => setShowInfoPanel(false)}
        photoInfo={photoInfo}
      />

      {/* Save/Discard prompt when leaving photo in edit mode */}
      <LeavePrompt
        show={showLeavePrompt}
        onCancel={() => setShowLeavePrompt(false)}
        onDiscard={handleLeaveAndDiscard}
        onSave={handleLeaveAndSave}
      />
    </div>
  );
}
