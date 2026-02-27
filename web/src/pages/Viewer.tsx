import { useEffect, useRef, useState, useCallback } from "react";
import { useParams, useNavigate, useLocation } from "react-router-dom";
import { api } from "../api/client";
import { decrypt } from "../crypto/crypto";
import { useAuthStore } from "../store/auth";
import { db, type MediaType } from "../db";

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
}

// ── Viewer ────────────────────────────────────────────────────────────────────

export default function Viewer() {
  const { id } = useParams<{ id: string }>();
  const location = useLocation();
  const isPlainMode = location.pathname.startsWith("/photo/plain/");
  const navigate = useNavigate();

  const [mediaUrl, setMediaUrl] = useState<string | null>(null);
  const [filename, setFilename] = useState("");
  const [mimeType, setMimeType] = useState("image/jpeg");
  const [mediaType, setMediaType] = useState<MediaType>("photo");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

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
      api.photos.list({ limit: 500 }).then((res) => {
        const photo = res.photos.find((p) => p.id === id);
        if (photo) {
          setIsFavorite(!!photo.is_favorite);
          if (photo.crop_metadata) {
            try { setCropData(JSON.parse(photo.crop_metadata)); } catch { setCropData(null); }
          } else {
            setCropData(null);
          }
        }
      }).catch(() => {});
    } else {
      // Encrypted mode: load crop data from local IndexedDB
      db.photos.get(id).then((cached) => {
        if (cached?.cropData) {
          try { setCropData(JSON.parse(cached.cropData)); } catch { setCropData(null); }
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

  // ── Navigation state ──────────────────────────────────────────────────────
  const state = location.state as ViewerLocationState | null;
  const photoIds = state?.photoIds;
  const currentIndex = state?.currentIndex ?? -1;
  const hasPrev = photoIds != null && currentIndex > 0;
  const hasNext = photoIds != null && currentIndex >= 0 && currentIndex < photoIds.length - 1;

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

  // ── Swipe handling ──────────────────────────────────────────────────────
  const touchStartX = useRef<number | null>(null);
  const touchStartY = useRef<number | null>(null);
  const swiped = useRef(false);

  function handleTouchStart(e: React.TouchEvent) {
    touchStartX.current = e.touches[0].clientX;
    touchStartY.current = e.touches[0].clientY;
    swiped.current = false;
  }

  function handleTouchEnd(e: React.TouchEvent) {
    if (editMode) return;
    if (touchStartX.current === null || touchStartY.current === null || swiped.current) return;
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
    touchStartX.current = null;
    touchStartY.current = null;
  }

  // ── Load media on id change ───────────────────────────────────────────────
  useEffect(() => {
    if (!id) return;

    // Reset state for new photo
    setMediaUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
    setPreviewUrl((prev) => { if (prev) URL.revokeObjectURL(prev); return null; });
    setLoading(true);
    setError("");

    if (isPlainMode) {
      loadPlainMedia(id);
    } else {
      // Show cached thumbnail immediately for a live-preview feel
      db.photos.get(id).then((cached) => {
        if (cached?.thumbnailData) {
          const url = URL.createObjectURL(new Blob([cached.thumbnailData], { type: "image/jpeg" }));
          setPreviewUrl(url);
        }
      });
      loadEncryptedMedia(id);
    }
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

  /** Load a plain-mode photo — fetch with auth and create object URL */
  async function loadPlainMedia(photoId: string) {
    setLoading(true);
    setError("");
    try {
      // Fetch photo metadata to get filename and media type
      const res = await api.photos.list({ limit: 500 });
      const photo = res.photos.find((p) => p.id === photoId);
      if (photo) {
        setFilename(photo.filename);
        setMimeType(photo.mime_type);
        const resolved: MediaType =
          photo.media_type === "gif" ? "gif"
          : photo.media_type === "video" ? "video"
          : "photo";
        setMediaType(resolved);
      }

      // Fetch the file with auth headers and create a local object URL
      const { accessToken } = useAuthStore.getState();
      const headers: Record<string, string> = { "X-Requested-With": "SimplePhotos" };
      if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
      const fileRes = await fetch(api.photos.fileUrl(photoId), { headers });
      if (!fileRes.ok) throw new Error(`Failed to load photo: ${fileRes.status}`);
      const blob = await fileRes.blob();
      const url = URL.createObjectURL(blob);
      setMediaUrl(url);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load media");
    } finally {
      setLoading(false);
    }
  }

  /** Load an encrypted blob, decrypt, and display */
  async function loadEncryptedMedia(blobId: string) {
    setLoading(true);
    setError("");
    try {
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
          : "photo");
      setMediaType(resolvedType);

      // Decode base64 → Blob → Object URL
      const bytes = base64ToUint8Array(payload.data).buffer as ArrayBuffer;
      const blob = new Blob([bytes], { type: payload.mime_type });
      const url = URL.createObjectURL(blob);
      setMediaUrl(url);

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
          className="text-white hover:text-gray-300 text-sm flex items-center gap-1"
        >
          ← Back
        </button>
        <div className="flex gap-3 items-center">
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
            className="text-white hover:text-gray-300 text-sm"
            disabled={!mediaUrl}
          >
            Download
          </button>
          <button
            onClick={handleDelete}
            className="text-red-400 hover:text-red-300 text-sm"
          >
            Delete
          </button>
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
            className="max-w-full max-h-full object-contain"
            onLoad={computeCropZoom}
            style={{
              imageRendering: mediaType === "gif" ? "auto" : undefined,
              ...(cropData ? cropZoomStyle : {}),
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
        {mediaUrl && mediaType === "video" && (
          <video
            ref={videoRef}
            src={mediaUrl}
            controls
            playsInline
            autoPlay={false}
            className="max-w-full max-h-full"
            style={{ background: "black" }}
          >
            <p className="text-white text-sm">
              Your browser doesn't support this video format.
            </p>
          </video>
        )}
        </div>{/* end slide animation wrapper */}
      </div>

      {/* ── Edit panel (Crop / Brightness tabs + Save) ───────────────── */}
      {editMode && mediaUrl && mediaType === "photo" && (
        <div className="absolute bottom-0 left-0 right-0 z-30 bg-black/90 border-t border-white/10 px-4 py-3 space-y-3">
          {/* Tab switcher */}
          <div className="flex items-center justify-center gap-2">
            <button
              onClick={() => setEditTab("crop")}
              className={`px-4 py-1.5 rounded-full text-sm font-medium transition-colors ${
                editTab === "crop"
                  ? "bg-white text-black"
                  : "bg-white/10 text-white hover:bg-white/20"
              }`}
            >
              Crop
            </button>
            <button
              onClick={() => setEditTab("brightness")}
              className={`px-4 py-1.5 rounded-full text-sm font-medium transition-colors ${
                editTab === "brightness"
                  ? "bg-white text-black"
                  : "bg-white/10 text-white hover:bg-white/20"
              }`}
            >
              Brightness
            </button>
          </div>

          {/* Brightness slider */}
          {editTab === "brightness" && (
            <div className="flex items-center gap-3 max-w-sm mx-auto">
              <svg className="w-5 h-5 text-gray-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <circle cx="12" cy="12" r="4" />
              </svg>
              <input
                type="range"
                min={-100}
                max={100}
                value={brightness}
                onChange={(e) => setBrightness(Number(e.target.value))}
                className="flex-1 h-1.5 rounded-full appearance-none bg-white/20 accent-white cursor-pointer"
              />
              <svg className="w-5 h-5 text-yellow-300 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <circle cx="12" cy="12" r="4" />
                <path strokeLinecap="round" d="M12 2v2m0 16v2m-7.07-3.93l1.41-1.41m9.9-9.9l1.41-1.41M2 12h2m16 0h2M4.93 4.93l1.41 1.41m9.9 9.9l1.41 1.41" />
              </svg>
            </div>
          )}

          {/* Action buttons */}
          <div className="flex items-center justify-center gap-2">
            <button
              onClick={handleSaveEdit}
              className="px-5 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700 transition-colors"
            >
              Save
            </button>
            {cropData && (
              <button
                onClick={handleClearCrop}
                className="px-4 py-2 bg-gray-600 text-white rounded-lg text-sm font-medium hover:bg-gray-500 transition-colors"
              >
                Reset
              </button>
            )}
            <button
              onClick={() => setEditMode(false)}
              className="px-4 py-2 bg-gray-700 text-white rounded-lg text-sm font-medium hover:bg-gray-600 transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Bottom meta bar (shown when media is loaded, overlay) */}
      {mediaUrl && !editMode && (
        <div className={`absolute bottom-0 left-0 right-0 z-20 transition-opacity duration-300 ${
          showOverlay ? 'opacity-100' : 'opacity-0 pointer-events-none'
        } px-4 py-2 bg-black/60 text-gray-400 text-xs space-y-2`}>
          {/* Tags section — plain mode only */}
          {isPlainMode && (
            <div className="flex items-center gap-2 flex-wrap">
              {tags.map((tag) => (
                <span
                  key={tag}
                  className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-blue-600/30 text-blue-300 text-xs"
                >
                  {tag}
                  <button
                    onClick={() => handleRemoveTag(tag)}
                    className="hover:text-white ml-0.5"
                    title={`Remove tag "${tag}"`}
                  >
                    ✕
                  </button>
                </span>
              ))}

              {/* Add tag button / inline input */}
              {showTagInput ? (
                <div className="relative inline-flex items-center">
                  <input
                    ref={tagInputRef}
                    type="text"
                    value={tagInput}
                    onChange={(e) => setTagInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") handleAddTag();
                      if (e.key === "Escape") { setShowTagInput(false); setTagInput(""); }
                    }}
                    placeholder="tag name"
                    className="w-28 px-2 py-0.5 rounded bg-gray-700 text-white text-xs border border-gray-600 focus:outline-none focus:border-blue-500"
                  />
                  <button
                    onClick={handleAddTag}
                    className="ml-1 text-blue-400 hover:text-blue-300 text-xs font-medium"
                  >
                    Add
                  </button>
                  <button
                    onClick={() => { setShowTagInput(false); setTagInput(""); }}
                    className="ml-1 text-gray-500 hover:text-gray-300 text-xs"
                  >
                    ✕
                  </button>
                  {/* Suggestions dropdown */}
                  {tagInput && tagSuggestions.length > 0 && (
                    <div className="absolute bottom-full left-0 mb-1 bg-gray-800 border border-gray-600 rounded shadow-lg z-50 min-w-[8rem]">
                      {tagSuggestions.map((s) => (
                        <button
                          key={s}
                          className="block w-full text-left px-2 py-1 text-xs text-gray-300 hover:bg-gray-700 hover:text-white"
                          onClick={() => { setTagInput(s); }}
                        >
                          {s}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              ) : (
                <button
                  onClick={() => setShowTagInput(true)}
                  className="px-2 py-0.5 rounded-full border border-dashed border-gray-500 text-gray-400 hover:text-white hover:border-gray-300 text-xs transition-colors"
                >
                  + Tag
                </button>
              )}
            </div>
          )}
        </div>
      )}

      {/* Save/Discard prompt when leaving photo in edit mode */}
      {showLeavePrompt && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70">
          <div className="bg-gray-800 rounded-xl p-6 max-w-sm w-full mx-4 space-y-4">
            <h3 className="text-white text-lg font-semibold">Unsaved Changes</h3>
            <p className="text-gray-300 text-sm">You have unsaved edits. Would you like to save or discard them?</p>
            <div className="flex gap-3 justify-end">
              <button
                onClick={() => setShowLeavePrompt(false)}
                className="px-4 py-2 bg-gray-700 text-white rounded-lg text-sm font-medium hover:bg-gray-600 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleLeaveAndDiscard}
                className="px-4 py-2 bg-red-600 text-white rounded-lg text-sm font-medium hover:bg-red-700 transition-colors"
              >
                Discard
              </button>
              <button
                onClick={handleLeaveAndSave}
                className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700 transition-colors"
              >
                Save
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function base64ToUint8Array(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
