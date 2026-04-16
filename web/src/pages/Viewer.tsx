/**
 * Full-screen photo/video viewer page.
 *
 * Orchestrates encrypted media loading, swipe/zoom gestures,
 * photo preloading, crop/brightness/rotation editing, favorites,
 * and prev/next navigation via photoIds passed through location.state.
 */
import { useEffect, useRef, useState, useCallback } from "react";
import { useParams, useNavigate, useLocation } from "react-router-dom";
import { db } from "../db";
import PhotoInfoPanel from "../components/viewer/PhotoInfoPanel";
import ViewerEditPanel from "../components/viewer/ViewerEditPanel";
import LeavePrompt from "../components/viewer/LeavePrompt";
import CropOverlay from "../components/viewer/CropOverlay";
import VideoControls from "../components/viewer/VideoControls";
import ViewerTopBar from "../components/viewer/ViewerTopBar";
import DownloadChoiceModal from "../components/viewer/DownloadChoiceModal";
import useZoomPan from "../hooks/useZoomPan";
import usePhotoPreload from "../hooks/usePhotoPreload";
import useViewerMedia from "../hooks/useViewerMedia";
import useViewerActions from "../hooks/useViewerActions";
import useViewerEdit from "../hooks/useViewerEdit";
import useSwipeNavigation from "../hooks/useSwipeNavigation";
import { useIsBackupServer } from "../hooks/useIsBackupServer";
import { diagnosticLogger } from "../utils/diagnosticLogger";
import type { PhotoInfoData } from "../hooks/useViewerMedia";

// ── Navigation context passed via location.state ─────────────────────────────
interface ViewerLocationState {
  /** Array of photo IDs in display order (for prev/next navigation) */
  photoIds?: string[];
  /** Current index within the photoIds array */
  currentIndex?: number;
  /** When viewing from an album, the album ID (enables "Remove" instead of "Delete") */
  albumId?: string;
  /** When true, the photo was opened from a secure gallery — hide all mutating UI */
  secureGallery?: boolean;
}

// ── Viewer ────────────────────────────────────────────────────────────────────

export default function Viewer() {
  const { id } = useParams<{ id: string }>();
  const location = useLocation();
  const navigate = useNavigate();
  const isBackupServer = useIsBackupServer();

  // Destructure navigation context from location.state (passed by Gallery)
  const navState = (location.state ?? {}) as ViewerLocationState;
  const photoIds = navState.photoIds;
  const currentIndex = navState.currentIndex ?? 0;
  const albumId = navState.albumId;
  const secureGallery = navState.secureGallery ?? false;
  const hasPrev = !!photoIds && currentIndex > 0;
  const hasNext = !!photoIds && currentIndex < photoIds.length - 1;

  // ── Favorite state ────────────────────────────────────────────────────────
  const [isFavorite, setIsFavorite] = useState(false);

  // ── Info panel state ───────────────────────────────────────────────────────
  const [showInfoPanel, setShowInfoPanel] = useState(false);
  const [photoInfo, setPhotoInfo] = useState<PhotoInfoData | null>(null);

  // ── Slide animation direction ─────────────────────────────────────────────
  const [slideDirection, setSlideDirection] = useState<"left" | "right" | null>(null);
  const [slideKey, setSlideKey] = useState(0);

  // ── Full-screen overlay state ──────────────────────────────────────────
  const [showOverlay, setShowOverlay] = useState(true);
  const viewerContainerRef = useRef<HTMLDivElement>(null);

  // ── Edit state (from hook) ─────────────────────────────────────────────
  const {
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
    cropImageRef, cropContainerRef, audioRef, videoRef, viewImgRef,
    resetEditState,
    computeRotationScale,
    computeCropZoom,
    enterEditMode,
    handleCornerPointerDown, handleCornerPointerMove, handleCornerPointerUp,
  } = useViewerEdit(viewerContainerRef);

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
  const { preloadCache, preloadAdjacentPhotos } = usePhotoPreload(
    photoIds, currentIndex,
  );

  // ── Media loading (from hook) ──────────────────────────────────────────
  const {
    mediaUrl, setMediaUrl,
    previewUrl, setPreviewUrl,
    filename, setFilename,
    mimeType, setMimeType,
    mediaType, setMediaType,
    loading, setLoading,
    error, setError,
    videoError, setVideoError,
    loadEncryptedMedia,
  } = useViewerMedia(preloadCache);

  // ── Actions (from hook) ────────────────────────────────────────────────
  const {
    showLeavePrompt, setShowLeavePrompt,
    saveCopySuccess,
    handleSaveEdit, handleSaveCopy, handleClearCrop,
    handleLeaveAndSave, handleLeaveAndDiscard,
    handleDelete, handleRemoveFromAlbum,
    handleDownload, handleDownloadOriginal,
    handleDownloadConverted, handleDownloadSource,
    handleToggleFavorite,
    isRenderingVideo,
    showDownloadChoice, setShowDownloadChoice,
  } = useViewerActions({
    id, mediaUrl, filename, mediaType, mimeType,
    albumId, photoIds, currentIndex,
    cropCorners, brightness, rotateValue, trimStart, trimEnd, mediaDuration,
    cropData, setCropData, setCropCorners, setBrightness, setRotateValue, setTrimStart, setTrimEnd,
    setEditMode, setError,
    preloadCache,
  });

  // ── Load favorite state + info for current photo ──────────────────────────
  useEffect(() => {
    if (!id) return;
    setIsFavorite(false);
    db.photos.get(id).then(async (cached) => {
      if (cached) {
        setIsFavorite(cached.isFavorite ?? false);
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
  }, [id]);

  async function onToggleFavorite() {
    const result = await handleToggleFavorite();
    if (result !== undefined) setIsFavorite(result);
  }

  // ── Navigation ─────────────────────────────────────────────────────────
  const navigateToPhoto = useCallback((index: number) => {
    if (!photoIds || index < 0 || index >= photoIds.length) return;
    const nextId = photoIds[index];
    setSlideDirection(index > currentIndex ? "right" : "left");
    setSlideKey((k) => k + 1);
    navigate(`/photo/${nextId}`, {
      replace: true,
      state: { photoIds, currentIndex: index, albumId, secureGallery } satisfies ViewerLocationState,
    });
  }, [photoIds, navigate, currentIndex]);

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

    // Reset all edit / playback state so nothing leaks across photos
    resetEditState();

    const cached = preloadCache.current.get(id);
    if (cached) {
      setMediaUrl((prev) => {
        if (prev && !Array.from(preloadCache.current.values()).some(e => e.url === prev)) URL.revokeObjectURL(prev);
        return cached.url;
      });
      setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
      setFilename(cached.filename);
      setMimeType(cached.mimeType);
      setMediaType(cached.mediaType);
      setCropData(cached.cropData ?? null);
      setIsFavorite(cached.isFavorite);
      setLoading(false);
      setError("");
      setVideoError(false);
    } else {
      setMediaUrl((prev) => {
        if (prev && !Array.from(preloadCache.current.values()).some(e => e.url === prev)) URL.revokeObjectURL(prev);
        return null;
      });
      setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
      setCropData(null);
      setFilename("");
      setLoading(true);
      setError("");
      setVideoError(false);
      db.photos.get(id).then((dbCached) => {
        if (dbCached?.thumbnailData) {
          const mime = dbCached.thumbnailMimeType || (dbCached.mediaType === "gif" ? "image/gif" : "image/jpeg");
          const url = URL.createObjectURL(new Blob([dbCached.thumbnailData], { type: mime }));
          setPreviewUrl(url);
        }
        // Use storageBlobId for copies that reference the original's server blob
        const fetchId = dbCached?.storageBlobId || id;
        diagnosticLogger.debug("VIEWER", `Resolved fetchId=${fetchId}`, { storageBlobId: dbCached?.storageBlobId });
        loadEncryptedMedia(fetchId);
      }).catch(() => loadEncryptedMedia(id));
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

  // ── Render ────────────────────────────────────────────────────────────────
  return (
    <div
      className="fixed inset-0 bg-black select-none"
      onTouchStart={handleTouchStart}
      onTouchMove={handleTouchMove}
      onTouchEnd={handleTouchEnd}
    >
      {/* Top bar (overlay) */}
      <ViewerTopBar
        editMode={editMode}
        showOverlay={showOverlay}
        showInfoPanel={showInfoPanel}
        setShowInfoPanel={setShowInfoPanel}
        mediaType={mediaType}
        mediaUrl={mediaUrl}
        isFavorite={isFavorite}
        isBackupServer={isBackupServer || secureGallery}
        isRenderingVideo={isRenderingVideo}
        albumId={albumId}
        onBack={() => { if (editMode) setShowLeavePrompt(true); else navigate(secureGallery ? "/secure-gallery" : "/gallery"); }}
        onToggleEdit={() => { if (editMode) setEditMode(false); else enterEditMode(mediaType); }}
        onToggleFavorite={onToggleFavorite}
        onDownload={handleDownload}
        onDelete={handleDelete}
        onRemoveFromAlbum={handleRemoveFromAlbum}
      />

      {/* Converting banner — shown while ffmpeg renders a video/audio file */}
      {isRenderingVideo && (
        <div className="absolute top-14 left-1/2 -translate-x-1/2 z-40 flex items-center gap-2 px-4 py-2 rounded-full bg-black/80 text-white text-sm shadow-lg pointer-events-none">
          <div className="w-3.5 h-3.5 border-2 border-white/40 border-t-white rounded-full animate-spin flex-shrink-0" />
          Converting… download will begin automatically
        </div>
      )}

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
        {/* Photo / GIF viewer — wrapper div always mounted so <img> stays in the
             same React tree position across normal ↔ edit transitions, preventing
             blob-URL reload failures (which caused BMP blanking & alt-text filenames) */}
        {mediaUrl && (mediaType === "photo" || mediaType === "gif") && (() => {
          const inEdit = editMode && mediaType === "photo";
          const rot = inEdit ? ((rotateValue % 360) + 360) % 360 : 0;
          const isSwapped = rot === 90 || rot === 270;

          return (
            <div
              ref={cropContainerRef}
              className={inEdit
                ? "relative w-full h-full flex items-center justify-center overflow-hidden"
                : "w-full h-full flex items-center justify-center"
              }
              onPointerMove={inEdit && editTab === "crop" ? (e: React.PointerEvent) => handleCornerPointerMove(e, mediaType) : undefined}
              onPointerUp={inEdit && editTab === "crop" ? handleCornerPointerUp : undefined}
            >
              <img
                ref={(el) => {
                  (viewImgRef as React.MutableRefObject<HTMLImageElement | null>).current = el;
                  (cropImageRef as React.MutableRefObject<HTMLImageElement | null>).current = el;
                }}
                src={mediaUrl}
                alt={filename}
                className={inEdit
                  ? "w-full h-full object-contain pointer-events-none"
                  : "w-full h-full object-contain transition-transform duration-150"
                }
                draggable={inEdit ? false : undefined}
                onLoad={inEdit ? undefined : computeCropZoom}
                style={{
                  imageRendering: mediaType === "gif" ? "auto" : undefined,
                  ...(inEdit ? {
                    filter: brightness !== 0 ? `brightness(${1 + brightness / 100})` : undefined,
                    ...(rot !== 0 ? {
                      transform: `rotate(${rot}deg)${isSwapped ? ` scale(${computeRotationScale(cropImageRef.current, cropContainerRef.current)})` : ""}`,
                    } : {}),
                  } : {
                    ...(cropData && zoomScale <= 1 ? cropZoomStyle : {}),
                    ...(zoomScale > 1 ? {
                      transform: `scale(${zoomScale}) translate(${panOffset.x / zoomScale}px, ${panOffset.y / zoomScale}px)`,
                      transformOrigin: `${zoomOrigin.x}% ${zoomOrigin.y}%`,
                      cursor: "grab",
                    } : {}),
                  }),
                }}
              />
              {inEdit && editTab === "crop" && cropImageRef.current && cropContainerRef.current && (
                <CropOverlay
                  mediaRect={cropImageRef.current.getBoundingClientRect()}
                  containerRect={cropContainerRef.current.getBoundingClientRect()}
                  cropCorners={cropCorners}
                  onCornerPointerDown={handleCornerPointerDown}
                />
              )}
            </div>
          );
        })()}

        {/* Video player (normal mode) — rotation applied to inner wrapper,
            custom controls sit outside so they stay upright */}
        {mediaUrl && mediaType === "video" && !videoError && !editMode && (() => {
          const rot = (((cropData?.rotate ?? 0) % 360) + 360) % 360;
          const isSwapped = rot === 90 || rot === 270;
          const rotStyle: React.CSSProperties = rot !== 0
            ? { transform: `rotate(${rot}deg)${isSwapped ? ` scale(${computeRotationScale(videoRef.current, viewerContainerRef.current)})` : ""}` }
            : {};
          // When crop data is present, use the full crop zoom style (includes
          // translate + scale + rotate + brightness), matching the photo behavior.
          // Otherwise fall back to simple rotation styling.
          const hasCropZoom = cropData && Object.keys(cropZoomStyle).length > 0;
          const wrapperStyle: React.CSSProperties = hasCropZoom ? cropZoomStyle : rotStyle;
          return (
          <div className="relative max-w-full max-h-full w-full h-full flex items-center justify-center">
            {/* Rotation/crop wrapper — only the video is transformed */}
            <div
              className="max-w-full max-h-full w-full h-full flex items-center justify-center"
              style={wrapperStyle}
            >
              <video
                ref={videoRef}
                src={mediaUrl}
                playsInline autoPlay
                className="w-full h-full"
                style={{
                  objectFit: 'contain' as const,
                  background: "black",
                  // Apply brightness only when NOT using cropZoomStyle (which includes its own filter)
                  ...(!hasCropZoom && cropData?.brightness ? { filter: `brightness(${1 + (cropData.brightness ?? 0) / 100})` } : {}),
                }}
                onLoadedMetadata={(e) => {
                  const v = e.currentTarget;
                  setMediaDuration(v.duration || 0);
                  if (cropData?.trimStart && cropData.trimStart > 0) v.currentTime = cropData.trimStart;
                  // Recompute crop zoom now that video dimensions are known
                  computeCropZoom();
                }}
                onTimeUpdate={(e) => {
                  if (cropData?.trimEnd && e.currentTarget.currentTime >= cropData.trimEnd) {
                    e.currentTarget.pause();
                    e.currentTarget.currentTime = cropData.trimEnd;
                  }
                }}
                onError={() => setVideoError(true)}
              />
            </div>
            {/* Custom controls — NOT rotated */}
            <VideoControls videoRef={videoRef} visible={showOverlay} />
          </div>
          );
        })()}

        {/* Video edit mode — rotation applied to wrapper div, custom controls outside
            so they stay upright (same pattern as normal mode) */}
        {editMode && mediaUrl && mediaType === "video" && !videoError && (() => {
          const rot = ((rotateValue % 360) + 360) % 360;
          const isSwapped = rot === 90 || rot === 270;
          const rotStyle: React.CSSProperties = rot !== 0
            ? { transform: `rotate(${rot}deg)${isSwapped ? ` scale(${computeRotationScale(videoRef.current, cropContainerRef.current)})` : ""}` }
            : {};
          return (
          <div
            ref={cropContainerRef}
            className="relative w-full h-full flex items-center justify-center overflow-hidden"
            onPointerMove={editTab === "crop" ? (e: React.PointerEvent) => handleCornerPointerMove(e, mediaType) : undefined}
            onPointerUp={editTab === "crop" ? handleCornerPointerUp : undefined}
          >
            {/* Rotation wrapper — only the video is rotated */}
            <div
              className="max-w-full max-h-full w-full h-full flex items-center justify-center"
              style={rotStyle}
            >
              <video
                ref={videoRef}
                src={mediaUrl}
                playsInline autoPlay={false}
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
            </div>
            {/* Custom controls — NOT rotated (sits outside rotation wrapper) */}
            <VideoControls videoRef={videoRef} visible={true} />
            {editTab === "crop" && videoRef.current && cropContainerRef.current && (
              <CropOverlay
                mediaRect={videoRef.current.getBoundingClientRect()}
                containerRect={cropContainerRef.current.getBoundingClientRect()}
                cropCorners={cropCorners}
                onCornerPointerDown={handleCornerPointerDown}
              />
            )}
          </div>
          );
        })()}

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
              controls autoPlay
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
          rotateValue={rotateValue} setRotateValue={setRotateValue}
          cropData={cropData} trimStart={trimStart} trimEnd={trimEnd}
          setTrimStart={setTrimStart} setTrimEnd={setTrimEnd} duration={mediaDuration}
          onSave={handleSaveEdit} onSaveCopy={handleSaveCopy}
          onClear={handleClearCrop} onCancel={() => setEditMode(false)}
        />
      )}

      {/* Photo info panel */}
      <PhotoInfoPanel show={showInfoPanel} onClose={() => setShowInfoPanel(false)} photoInfo={photoInfo} />
      <LeavePrompt show={showLeavePrompt} onCancel={() => setShowLeavePrompt(false)} onDiscard={handleLeaveAndDiscard} onSave={handleLeaveAndSave} />

      {/* Download choice dialog for converted files */}
      {showDownloadChoice && (
        <DownloadChoiceModal
          onConvertedDownload={handleDownloadConverted}
          onSourceDownload={handleDownloadSource}
          onCancel={() => setShowDownloadChoice(false)}
        />
      )}

      {/* Save Copy success toast */}
      {saveCopySuccess && (
        <div className="fixed top-16 left-1/2 -translate-x-1/2 z-50 bg-green-600 text-white px-4 py-2 rounded-lg shadow-lg text-sm font-medium animate-fade-in">
          Copy saved ✓
        </div>
      )}

    </div>
  );
}
