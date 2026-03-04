import { useEffect, useRef, useState, useCallback } from "react";
import { useParams, useNavigate, useLocation } from "react-router-dom";
import { api } from "../api/client";
import { db, type MediaType } from "../db";
import AppIcon from "../components/AppIcon";
import PhotoInfoPanel from "../components/viewer/PhotoInfoPanel";
import ViewerEditPanel, { type EditTab } from "../components/viewer/ViewerEditPanel";
import TagsBar from "../components/viewer/TagsBar";
import LeavePrompt from "../components/viewer/LeavePrompt";
import DownloadFormatDialog from "../components/viewer/DownloadFormatDialog";
import CropOverlay from "../components/viewer/CropOverlay";
import useZoomPan from "../hooks/useZoomPan";
import usePhotoPreload from "../hooks/usePhotoPreload";
import useViewerMedia from "../hooks/useViewerMedia";
import useViewerActions from "../hooks/useViewerActions";
import useSwipeNavigation from "../hooks/useSwipeNavigation";
import type { CropMetadata, PhotoInfoData } from "../hooks/useViewerMedia";

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
  const [photoInfo, setPhotoInfo] = useState<PhotoInfoData | null>(null);

  // ── Slide animation direction ─────────────────────────────────────────────
  const [slideDirection, setSlideDirection] = useState<"left" | "right" | null>(null);
  const [slideKey, setSlideKey] = useState(0);

  // ── Edit mode state ────────────────────────────────────────────────────
  const [editMode, setEditMode] = useState(false);
  const [editTab, setEditTab] = useState<EditTab>("crop");
  const [cropData, setCropData] = useState<CropMetadata | null>(null);
  const [cropCorners, setCropCorners] = useState<{ x: number; y: number; w: number; h: number }>({ x: 0, y: 0, w: 1, h: 1 });
  const [draggingCorner, setDraggingCorner] = useState<string | null>(null);
  const [brightness, setBrightness] = useState(0);
  const [trimStart, setTrimStart] = useState(0);
  const [trimEnd, setTrimEnd] = useState(0);
  const [mediaDuration, setMediaDuration] = useState(0);
  const cropImageRef = useRef<HTMLImageElement>(null);
  const cropContainerRef = useRef<HTMLDivElement>(null);
  const audioRef = useRef<HTMLAudioElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);

  // ── Full-screen overlay state ──────────────────────────────────────────
  const [showOverlay, setShowOverlay] = useState(true);
  const viewerContainerRef = useRef<HTMLDivElement>(null);
  const viewImgRef = useRef<HTMLImageElement>(null);
  const [cropZoomStyle, setCropZoomStyle] = useState<React.CSSProperties>({});

  // ── Zoom state (from hook) ─────────────────────────────────────────────
  const {
    zoomScale, setZoomScale,
    zoomOrigin, setZoomOrigin,
    panOffset, setPanOffset,
    lastTapTime, pinchStartDist, pinchStartScale, panStart,
    handleDoubleClickZoom,
    handleWheel,
  } = useZoomPan(id, editMode, viewerContainerRef);

  // ── Preload cache for adjacent photos ──────────────────────────────────
  const { preloadCache, getCachedPhotoList, preloadAdjacentPhotos } = usePhotoPreload(
    photoIds, currentIndex, isPlainMode,
  );

  // ── Media loading (from hook) ──────────────────────────────────────────
  const {
    mediaUrl, setMediaUrl,
    previewUrl, setPreviewUrl,
    filename, mimeType, mediaType,
    loading, setLoading,
    error, setError,
    videoError, setVideoError,
    isConverting,
    loadPlainMedia, loadEncryptedMedia,
  } = useViewerMedia(getCachedPhotoList, preloadCache);

  // ── Actions (from hook) ────────────────────────────────────────────────
  const {
    showLeavePrompt, setShowLeavePrompt,
    showDownloadDialog, setShowDownloadDialog,
    handleSaveEdit, handleSaveCopy, handleClearCrop,
    handleLeaveAndSave, handleLeaveAndDiscard,
    handleDelete, handleRemoveFromAlbum,
    handleDownload, handleDownloadOriginal, handleDownloadConverted,
    handleToggleFavorite,
  } = useViewerActions({
    id, isPlainMode, mediaUrl, filename, mediaType,
    albumId, photoIds, currentIndex,
    cropCorners, brightness, trimStart, trimEnd, mediaDuration,
    setCropData, setCropCorners, setBrightness, setTrimStart, setTrimEnd,
    setEditMode, setError,
    preloadCache,
  });

  // ── Crop zoom transform ────────────────────────────────────────────────
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

  useEffect(() => {
    computeCropZoom();
    window.addEventListener("resize", computeCropZoom);
    return () => window.removeEventListener("resize", computeCropZoom);
  }, [computeCropZoom]);

  // ── Load tags + favorite state for current photo ─────────────────────────
  useEffect(() => {
    if (!id) return;
    setTags([]);
    setIsFavorite(false);
    if (isPlainMode) {
      api.tags.getPhotoTags(id).then((res) => setTags(res.tags)).catch(() => {});
      api.tags.list().then((res) => setAllTags(res.tags)).catch(() => {});
      getCachedPhotoList().then((photos) => {
        const photo = photos.find((p) => p.id === id);
        if (photo) {
          setIsFavorite(!!photo.is_favorite);
          setPhotoInfo({
            filename: photo.filename, mimeType: photo.mime_type,
            width: photo.width, height: photo.height,
            takenAt: photo.taken_at, sizeBytes: photo.size_bytes,
            latitude: photo.latitude, longitude: photo.longitude,
            createdAt: photo.created_at, durationSecs: photo.duration_secs,
            cameraModel: photo.camera_model,
          });
          if (photo.crop_metadata) {
            try { setCropData(JSON.parse(photo.crop_metadata)); } catch { setCropData(null); }
          } else { setCropData(null); }
        }
      }).catch(() => {});
    } else {
      db.photos.get(id).then(async (cached) => {
        if (cached) {
          const allAlbums = await db.albums.toArray();
          const albumNames = allAlbums.filter((a) => a.photoBlobIds.includes(id!)).map((a) => a.name);
          setPhotoInfo({
            filename: cached.filename, mimeType: cached.mimeType,
            width: cached.width, height: cached.height,
            takenAt: cached.takenAt ? new Date(cached.takenAt).toISOString() : null,
            latitude: cached.latitude, longitude: cached.longitude,
            albumNames,
          });
          if (cached.cropData) {
            try { setCropData(JSON.parse(cached.cropData)); } catch { setCropData(null); }
          } else { setCropData(null); }
        } else { setCropData(null); }
      }).catch(() => { setCropData(null); });
    }
  }, [id, isPlainMode]);

  // Auto-focus tag input when shown
  useEffect(() => { if (showTagInput) tagInputRef.current?.focus(); }, [showTagInput]);

  async function handleAddTag() {
    const tag = tagInput.trim().toLowerCase();
    if (!tag || !id) return;
    try {
      await api.tags.add(id, tag);
      setTags((prev) => (prev.includes(tag) ? prev : [...prev, tag].sort()));
      if (!allTags.includes(tag)) setAllTags((prev) => [...prev, tag].sort());
      setTagInput("");
    } catch { /* ignore */ }
  }

  async function handleRemoveTag(tag: string) {
    if (!id) return;
    try {
      await api.tags.remove(id, tag);
      setTags((prev) => prev.filter((t) => t !== tag));
    } catch { /* ignore */ }
  }

  async function onToggleFavorite() {
    const result = await handleToggleFavorite();
    if (result !== undefined) setIsFavorite(result);
  }

  // Initialize edit state from existing metadata when entering edit mode
  function enterEditMode() {
    // Probe the actual element duration as a fallback — mediaDuration may
    // still be 0 if onLoadedMetadata hasn't fired yet.
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
      setTrimStart(cropData.trimStart ?? 0);
      setTrimEnd(cropData.trimEnd ?? dur);
    } else {
      setCropCorners({ x: 0, y: 0, w: 1, h: 1 });
      setBrightness(0);
      setTrimStart(0);
      setTrimEnd(dur);
    }
    setEditTab(mediaType === "audio" || mediaType === "video" ? "trim" : "brightness");
    setEditMode(true);
  }

  // ── Navigation ─────────────────────────────────────────────────────────
  const navigateToPhoto = useCallback((index: number) => {
    if (!photoIds || index < 0 || index >= photoIds.length) return;
    const nextId = photoIds[index];
    const prefix = isPlainMode ? "/photo/plain/" : "/photo/";
    setSlideDirection(index > currentIndex ? "right" : "left");
    setSlideKey((k) => k + 1);
    navigate(`${prefix}${nextId}`, {
      replace: true,
      state: { photoIds, currentIndex: index } satisfies ViewerLocationState,
    });
  }, [photoIds, isPlainMode, navigate, currentIndex]);

  const goPrev = useCallback(() => { if (hasPrev) navigateToPhoto(currentIndex - 1); }, [hasPrev, currentIndex, navigateToPhoto]);
  const goNext = useCallback(() => { if (hasNext) navigateToPhoto(currentIndex + 1); }, [hasNext, currentIndex, navigateToPhoto]);

  // ── Swipe / pinch / double-tap (from hook) ─────────────────────────────
  const { swiped, handleTouchStart, handleTouchMove, handleTouchEnd } = useSwipeNavigation({
    editMode, zoomScale,
    setZoomScale: (fn) => setZoomScale(fn(zoomScale)),
    setZoomOrigin, panOffset, setPanOffset,
    pinchStartDist, pinchStartScale, panStart, lastTapTime,
    viewerContainerRef, goPrev, goNext,
    showInfoPanel, setShowInfoPanel,
    navigateBack: () => navigate(-1),
  });

  // ── Load media on id change (with preload cache) ───────────────────────
  useEffect(() => {
    if (!id) return;
    const cached = preloadCache.current.get(id);
    if (cached) {
      setMediaUrl((prev) => {
        if (prev && !Array.from(preloadCache.current.values()).some(e => e.url === prev)) URL.revokeObjectURL(prev);
        return cached.url;
      });
      setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
      setLoading(false);
      setError("");
      setVideoError(false);
    } else {
      setMediaUrl((prev) => {
        if (prev && !Array.from(preloadCache.current.values()).some(e => e.url === prev)) URL.revokeObjectURL(prev);
        return null;
      });
      setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
      setLoading(true);
      setError("");
      setVideoError(false);
      if (isPlainMode) loadPlainMedia(id);
      else {
        db.photos.get(id).then((dbCached) => {
          if (dbCached?.thumbnailData) {
            const url = URL.createObjectURL(new Blob([dbCached.thumbnailData], { type: "image/jpeg" }));
            setPreviewUrl(url);
          }
        });
        loadEncryptedMedia(id);
      }
    }
    const preloadTimer = setTimeout(() => preloadAdjacentPhotos(id), 50);
    return () => clearTimeout(preloadTimer);
  }, [id]);

  // ── Keyboard navigation ────────────────────────────────────────────────
  useEffect(() => {
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (showLeavePrompt) setShowLeavePrompt(false);
        else if (editMode) setShowLeavePrompt(true);
        else navigate("/gallery");
        return;
      }
      if (editMode) return;
      if (e.key === "ArrowLeft") goPrev();
      if (e.key === "ArrowRight") goNext();
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [goPrev, goNext, navigate, editMode, showLeavePrompt]);

  // ── Corner drag handlers ────────────────────────────────────────────────
  function getMediaRect() {
    if (mediaType === "video" && videoRef.current) return videoRef.current.getBoundingClientRect();
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
    const imgRect = getMediaRect();
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
  }

  function handleCornerPointerUp() { setDraggingCorner(null); }

  const tagSuggestions = allTags.filter(
    (t) => !tags.includes(t) && t.includes(tagInput.toLowerCase())
  ).slice(0, 5);

  // ── Render ────────────────────────────────────────────────────────────────
  return (
    <div
      className="fixed inset-0 bg-black select-none"
      onTouchStart={handleTouchStart}
      onTouchMove={handleTouchMove}
      onTouchEnd={handleTouchEnd}
    >
      {/* Top bar (overlay) */}
      <div className={`absolute top-0 left-0 right-0 z-30 transition-opacity duration-300 ${
        showOverlay || editMode ? "opacity-100" : "opacity-0 pointer-events-none"
      }`}>
      <div className="flex items-center justify-between px-4 py-3 bg-black/80">
        <button
          onClick={() => { if (editMode) setShowLeavePrompt(true); else navigate("/gallery"); }}
          className="text-white hover:text-gray-300 flex items-center justify-center w-8 h-8 rounded-full hover:bg-white/20 transition-colors"
          title="Back"
        >
          <AppIcon name="back-arrow" size="w-5 h-5" themed={false} className="invert" />
        </button>
        <div className="flex gap-3 items-center">
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
          {(mediaType === "photo" || mediaType === "video" || mediaType === "audio") && (
            <button
              onClick={() => { if (editMode) setEditMode(false); else enterEditMode(); }}
              className={`flex items-center gap-1 px-2 py-1 rounded text-sm font-medium transition-colors ${
                editMode ? "bg-blue-600 text-white" : "text-white hover:bg-white/20"
              }`}
              title="Edit"
            >Edit</button>
          )}
          {isPlainMode && (
            <button
              onClick={onToggleFavorite}
              className={`hover:scale-110 transition-transform ${isFavorite ? "text-yellow-400" : "text-white hover:text-yellow-300"}`}
              title={isFavorite ? "Unfavorite" : "Favorite"}
            >
              {isFavorite ? (
                <svg className="w-5 h-5" viewBox="0 0 24 24" fill="currentColor"><path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" /></svg>
              ) : (
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z" /></svg>
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
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M15 12H9m12 0a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
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
          if ((e.target as HTMLElement).closest("button")) return;
          if (!editMode) setShowOverlay(prev => !prev);
        }}
        onDoubleClick={handleDoubleClickZoom}
        onWheel={handleWheel}
      >
        {/* Live preview: blurred thumbnail shown while full media loads */}
        {previewUrl && loading && (
          <img src={previewUrl} alt="preview" className="absolute inset-0 w-full h-full object-contain blur-sm opacity-60 pointer-events-none" />
        )}
        {loading && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="w-8 h-8 border-2 border-white/30 border-t-white rounded-full animate-spin" />
          </div>
        )}
        {error && <p className="text-red-400 text-sm z-10">{error}</p>}

        {/* Previous / Next arrows */}
        {hasPrev && !editMode && (
          <button onClick={goPrev} className="absolute left-2 top-1/2 -translate-y-1/2 z-20 w-10 h-10 md:w-12 md:h-12 flex items-center justify-center rounded-full bg-black/50 hover:bg-black/80 text-white transition-colors" aria-label="Previous photo">
            <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M15.75 19.5L8.25 12l7.5-7.5" /></svg>
          </button>
        )}
        {hasNext && !editMode && (
          <button onClick={goNext} className="absolute right-2 top-1/2 -translate-y-1/2 z-20 w-10 h-10 md:w-12 md:h-12 flex items-center justify-center rounded-full bg-black/50 hover:bg-black/80 text-white transition-colors" aria-label="Next photo">
            <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M8.25 4.5l7.5 7.5-7.5 7.5" /></svg>
          </button>
        )}

        {/* Slide animation wrapper */}
        <div
          key={slideKey}
          className={`w-full h-full flex items-center justify-center ${
            slideDirection === "right" ? "slide-in-right" : slideDirection === "left" ? "slide-in-left" : ""
          }`}
        >
        {/* Photo / GIF viewer (normal mode) */}
        {mediaUrl && (mediaType === "photo" || mediaType === "gif") && !editMode && (
          <img
            ref={viewImgRef}
            src={mediaUrl}
            alt={filename}
            className="max-w-full max-h-full object-contain transition-transform duration-150"
            onLoad={computeCropZoom}
            style={{
              imageRendering: mediaType === "gif" ? "auto" : undefined,
              ...(mimeType === "image/svg+xml" ? { backgroundColor: "white" } : {}),
              ...(cropData && zoomScale <= 1 ? cropZoomStyle : {}),
              ...(zoomScale > 1 ? {
                transform: `scale(${zoomScale}) translate(${panOffset.x / zoomScale}px, ${panOffset.y / zoomScale}px)`,
                transformOrigin: `${zoomOrigin.x}% ${zoomOrigin.y}%`,
                cursor: "grab",
              } : {}),
            }}
          />
        )}

        {/* Photo edit mode */}
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
                ...(mimeType === "image/svg+xml" ? { backgroundColor: "white" } : {}),
              }}
            />
            {editTab === "crop" && cropImageRef.current && cropContainerRef.current && (
              <CropOverlay
                mediaRect={cropImageRef.current.getBoundingClientRect()}
                containerRect={cropContainerRef.current.getBoundingClientRect()}
                cropCorners={cropCorners}
                onCornerPointerDown={handleCornerPointerDown}
              />
            )}
          </div>
        )}

        {/* Video player (normal mode) */}
        {mediaUrl && mediaType === "video" && !videoError && !editMode && (
          <video
            ref={videoRef}
            src={mediaUrl}
            controls playsInline autoPlay={false}
            className="max-w-full max-h-full"
            style={{
              background: "black",
              filter: cropData?.brightness ? `brightness(${1 + (cropData.brightness ?? 0) / 100})` : undefined,
            }}
            onLoadedMetadata={(e) => {
              const v = e.currentTarget;
              setMediaDuration(v.duration || 0);
              if (cropData?.trimStart && cropData.trimStart > 0) v.currentTime = cropData.trimStart;
            }}
            onTimeUpdate={(e) => {
              if (cropData?.trimEnd && e.currentTarget.currentTime >= cropData.trimEnd) {
                e.currentTarget.pause();
                e.currentTarget.currentTime = cropData.trimEnd;
              }
            }}
            onError={() => setVideoError(true)}
          />
        )}

        {/* Video edit mode */}
        {editMode && mediaUrl && mediaType === "video" && !videoError && (
          <div
            ref={cropContainerRef}
            className="relative w-full h-full flex items-center justify-center"
            onPointerMove={editTab === "crop" ? handleCornerPointerMove : undefined}
            onPointerUp={editTab === "crop" ? handleCornerPointerUp : undefined}
          >
            <video
              ref={videoRef}
              src={mediaUrl}
              controls playsInline autoPlay={false}
              className="max-w-full max-h-full pointer-events-auto"
              style={{
                background: "black",
                filter: brightness !== 0 ? `brightness(${1 + brightness / 100})` : undefined,
              }}
              onLoadedMetadata={(e) => {
                const v = e.currentTarget;
                const dur = v.duration || 0;
                setMediaDuration(dur);
                if (trimEnd <= 0 || trimEnd > dur) setTrimEnd(dur);
                if (trimStart > 0) v.currentTime = trimStart;
              }}
              onTimeUpdate={(e) => {
                if (editTab === "trim" && trimEnd > 0 && e.currentTarget.currentTime >= trimEnd) {
                  e.currentTarget.pause();
                  e.currentTarget.currentTime = trimEnd;
                }
              }}
            />
            {editTab === "crop" && videoRef.current && cropContainerRef.current && (
              <CropOverlay
                mediaRect={videoRef.current.getBoundingClientRect()}
                containerRect={cropContainerRef.current.getBoundingClientRect()}
                cropCorners={cropCorners}
                onCornerPointerDown={handleCornerPointerDown}
              />
            )}
          </div>
        )}

        {/* Conversion in progress — file not yet available in browser-compatible format */}
        {isConverting && !mediaUrl && !loading && (
          <div className="w-full h-full flex flex-col items-center justify-center" style={{ background: "black" }}>
            <div className="w-10 h-10 border-3 border-gray-600 border-t-blue-500 rounded-full animate-spin mb-4" />
            <p className="text-gray-300 text-sm mb-1">Converting to browser-compatible format...</p>
            <p className="text-gray-500 text-xs mb-4 px-4 text-center truncate max-w-[80%]">{filename}</p>
            <p className="text-gray-600 text-xs">This file will be viewable once conversion completes.</p>
          </div>
        )}

        {/* Video format not supported fallback */}
        {mediaUrl && mediaType === "video" && videoError && (
          <div className="w-full h-full flex flex-col items-center justify-center" style={{ background: "black" }}>
            <svg className="w-16 h-16 text-gray-500 mb-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="m15.75 10.5 4.72-4.72a.75.75 0 0 1 1.28.53v11.38a.75.75 0 0 1-1.28.53l-4.72-4.72M4.5 18.75h9a2.25 2.25 0 0 0 2.25-2.25v-9a2.25 2.25 0 0 0-2.25-2.25h-9A2.25 2.25 0 0 0 2.25 7.5v9a2.25 2.25 0 0 0 2.25 2.25Z" />
            </svg>
            <p className="text-gray-300 text-sm mb-1">This video format is not supported by your browser.</p>
            <p className="text-gray-500 text-xs mb-4 px-4 text-center truncate max-w-[80%]">{filename}</p>
            <button onClick={handleDownload} className="px-4 py-2 bg-blue-600 text-white text-sm rounded-lg hover:bg-blue-700 transition-colors">Download</button>
          </div>
        )}

        {/* Audio player (normal mode) */}
        {mediaUrl && mediaType === "audio" && !editMode && (
          <div className="w-full h-full flex flex-col items-center justify-center" style={{ background: "black" }}>
            <div className="text-gray-400 text-6xl mb-6">♫</div>
            <p className="text-gray-300 text-sm mb-6 px-4 text-center truncate max-w-[80%]">{filename}</p>
            <audio
              ref={audioRef}
              src={mediaUrl}
              controls autoPlay={false}
              className="w-full max-w-md"
              style={{ filter: "invert(1) hue-rotate(180deg)", opacity: 0.85 }}
              onLoadedMetadata={(e) => {
                const a = e.currentTarget;
                const dur = a.duration;
                if (dur && isFinite(dur) && dur > 0) setMediaDuration(dur);
                if (cropData?.trimStart && cropData.trimStart > 0) a.currentTime = cropData.trimStart;
              }}
              onDurationChange={(e) => {
                const dur = e.currentTarget.duration;
                if (dur && isFinite(dur) && dur > 0) setMediaDuration(dur);
              }}
              onTimeUpdate={(e) => {
                if (cropData?.trimEnd && e.currentTarget.currentTime >= cropData.trimEnd) {
                  e.currentTarget.pause();
                  e.currentTarget.currentTime = cropData.trimEnd;
                }
              }}
            />
          </div>
        )}

        {/* Audio edit mode */}
        {editMode && mediaUrl && mediaType === "audio" && (
          <div className="w-full h-full flex flex-col items-center justify-center" style={{ background: "black" }}>
            <div className="text-gray-400 text-6xl mb-6">♫</div>
            <p className="text-gray-300 text-sm mb-4 px-4 text-center truncate max-w-[80%]">{filename}</p>
            <p className="text-gray-500 text-xs mb-6">Adjust trim points below, then preview with the player</p>
            <audio
              ref={audioRef}
              src={mediaUrl}
              controls autoPlay={false}
              className="w-full max-w-md"
              style={{ filter: "invert(1) hue-rotate(180deg)", opacity: 0.85 }}
              onLoadedMetadata={(e) => {
                const a = e.currentTarget;
                const dur = a.duration;
                if (dur && isFinite(dur) && dur > 0) {
                  setMediaDuration(dur);
                  if (trimEnd <= 0 || trimEnd > dur) setTrimEnd(dur);
                }
                if (trimStart > 0) a.currentTime = trimStart;
              }}
              onDurationChange={(e) => {
                const dur = e.currentTarget.duration;
                if (dur && isFinite(dur) && dur > 0) {
                  setMediaDuration(dur);
                  if (trimEnd <= 0 || trimEnd > dur) setTrimEnd(dur);
                }
              }}
              onTimeUpdate={(e) => {
                if (trimEnd > 0 && e.currentTarget.currentTime >= trimEnd) {
                  e.currentTarget.pause();
                  e.currentTarget.currentTime = trimEnd;
                }
              }}
            />
          </div>
        )}
        </div>{/* end slide animation wrapper */}
      </div>

      {/* Edit panel */}
      {editMode && mediaUrl && (mediaType === "photo" || mediaType === "video" || mediaType === "audio") && (
        <ViewerEditPanel
          editTab={editTab} setEditTab={setEditTab}
          mediaType={mediaType} brightness={brightness} setBrightness={setBrightness}
          cropData={cropData} trimStart={trimStart} trimEnd={trimEnd}
          setTrimStart={setTrimStart} setTrimEnd={setTrimEnd} duration={mediaDuration}
          onSave={handleSaveEdit} onSaveCopy={handleSaveCopy}
          onClear={handleClearCrop} onCancel={() => setEditMode(false)}
        />
      )}

      {/* Bottom tags bar */}
      {mediaUrl && !editMode && (
        <TagsBar
          show={showOverlay} isPlainMode={isPlainMode} tags={tags}
          showTagInput={showTagInput} tagInput={tagInput}
          setTagInput={setTagInput} setShowTagInput={setShowTagInput}
          tagSuggestions={tagSuggestions} onAddTag={handleAddTag}
          onRemoveTag={handleRemoveTag} tagInputRef={tagInputRef}
        />
      )}

      <PhotoInfoPanel show={showInfoPanel} onClose={() => setShowInfoPanel(false)} photoInfo={photoInfo} />
      <LeavePrompt show={showLeavePrompt} onCancel={() => setShowLeavePrompt(false)} onDiscard={handleLeaveAndDiscard} onSave={handleLeaveAndSave} />
      {showDownloadDialog && (
        <DownloadFormatDialog
          filename={filename}
          onDownloadOriginal={handleDownloadOriginal}
          onDownloadConverted={handleDownloadConverted}
          onCancel={() => setShowDownloadDialog(false)}
        />
      )}
    </div>
  );
}
