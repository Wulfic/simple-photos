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

  // ── Crop/edit state ───────────────────────────────────────────────────────
  const [showCropEditor, setShowCropEditor] = useState(false);
  const [cropData, setCropData] = useState<{ x: number; y: number; width: number; height: number; rotate: number } | null>(null);
  const [cropDragging, setCropDragging] = useState(false);
  const [cropStart, setCropStart] = useState<{ x: number; y: number } | null>(null);
  const [cropRect, setCropRect] = useState<{ startX: number; startY: number; endX: number; endY: number } | null>(null);
  const cropImageRef = useRef<HTMLImageElement>(null);
  const cropContainerRef = useRef<HTMLDivElement>(null);

  const videoRef = useRef<HTMLVideoElement>(null);

  // ── Load tags + favorite state for current photo ─────────────────────────
  useEffect(() => {
    if (!id || !isPlainMode) return;
    setTags([]);
    setIsFavorite(false);
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

  async function handleSaveCrop() {
    if (!id || !isPlainMode) return;
    if (!cropRect || !cropImageRef.current || !cropContainerRef.current) return;
    const img = cropImageRef.current;
    const container = cropContainerRef.current;
    const containerRect = container.getBoundingClientRect();
    // Convert pixel coordinates to percentage of displayed image
    const imgRect = img.getBoundingClientRect();
    const x = (Math.min(cropRect.startX, cropRect.endX) - imgRect.left) / imgRect.width;
    const y = (Math.min(cropRect.startY, cropRect.endY) - imgRect.top) / imgRect.height;
    const w = Math.abs(cropRect.endX - cropRect.startX) / imgRect.width;
    const h = Math.abs(cropRect.endY - cropRect.startY) / imgRect.height;
    // Clamp values
    const crop = {
      x: Math.max(0, Math.min(1, x)),
      y: Math.max(0, Math.min(1, y)),
      width: Math.max(0.05, Math.min(1, w)),
      height: Math.max(0.05, Math.min(1, h)),
      rotate: 0,
    };
    try {
      await api.photos.setCrop(id, JSON.stringify(crop));
      setCropData(crop);
      setShowCropEditor(false);
      setCropRect(null);
    } catch {
      // ignore
    }
  }

  async function handleClearCrop() {
    if (!id || !isPlainMode) return;
    try {
      await api.photos.setCrop(id, null);
      setCropData(null);
      setShowCropEditor(false);
      setCropRect(null);
    } catch {
      // ignore
    }
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
      navigate(`${prefix}${nextId}`, {
        replace: true,
        state: { photoIds, currentIndex: index } satisfies ViewerLocationState,
      });
    },
    [photoIds, isPlainMode, navigate]
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
      if (e.key === "Escape") navigate("/gallery");
      if (e.key === "ArrowLeft") goPrev();
      if (e.key === "ArrowRight") goNext();
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [goPrev, goNext, navigate]);

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
    const msg = isPlainMode
      ? "Move this item to trash? You can restore it within 30 days."
      : "Permanently delete this item? This cannot be undone.";
    if (!id || !confirm(msg)) return;
    try {
      if (isPlainMode) {
        // Server soft-deletes to trash for plain mode
        await api.photos.delete(id);
      } else {
        await api.blobs.delete(id);
        const cached = await db.photos.get(id);
        if (cached?.thumbnailBlobId) {
          await api.blobs.delete(cached.thumbnailBlobId).catch(() => {});
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
      className="fixed inset-0 bg-black flex flex-col select-none"
      onTouchStart={handleTouchStart}
      onTouchEnd={handleTouchEnd}
    >
      {/* Top bar */}
      <div className="flex items-center justify-between px-4 py-3 bg-black/80 z-10">
        <button
          onClick={() => navigate("/gallery")}
          className="text-white hover:text-gray-300 text-sm flex items-center gap-1"
        >
          ← Back
        </button>
        <span className="text-white text-sm truncate mx-4 max-w-xs">{filename}</span>
        <div className="flex gap-3 items-center">
          {/* Crop/Edit button — plain mode photos only (not videos/gifs) */}
          {isPlainMode && mediaType === "photo" && (
            <button
              onClick={() => { setShowCropEditor(!showCropEditor); setCropRect(null); }}
              className="text-white hover:text-gray-300 text-sm"
              title="Edit / Crop"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M7 7h10v10M7 17V3m14 4H7m10 10v4" />
              </svg>
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

      {/* Content area */}
      <div className="flex-1 flex items-center justify-center overflow-hidden relative">
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
            <div className="text-white text-sm bg-black/50 px-4 py-2 rounded-full">
              {isPlainMode ? "Loading…" : "Decrypting…"}
            </div>
          </div>
        )}

        {error && (
          <p className="text-red-400 text-sm z-10">{error}</p>
        )}

        {/* ── Previous arrow (left side) ─────────────────────────────── */}
        {hasPrev && (
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
        {hasNext && (
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

        {/* ── Photo / GIF viewer ─────────────────────────────────────────── */}
        {mediaUrl && (mediaType === "photo" || mediaType === "gif") && !showCropEditor && (
          <img
            src={mediaUrl}
            alt={filename}
            className="max-w-full max-h-full object-contain"
            style={{
              imageRendering: mediaType === "gif" ? "auto" : undefined,
              ...(cropData ? {
                clipPath: `inset(${cropData.y * 100}% ${(1 - cropData.x - cropData.width) * 100}% ${(1 - cropData.y - cropData.height) * 100}% ${cropData.x * 100}%)`,
              } : {}),
            }}
          />
        )}

        {/* ── Crop editor overlay ────────────────────────────────────────── */}
        {showCropEditor && mediaUrl && mediaType === "photo" && (
          <div
            ref={cropContainerRef}
            className="relative w-full h-full flex items-center justify-center"
            onMouseDown={(e) => {
              setCropDragging(true);
              setCropStart({ x: e.clientX, y: e.clientY });
              setCropRect({ startX: e.clientX, startY: e.clientY, endX: e.clientX, endY: e.clientY });
            }}
            onMouseMove={(e) => {
              if (cropDragging && cropStart) {
                setCropRect((prev) => prev ? { ...prev, endX: e.clientX, endY: e.clientY } : null);
              }
            }}
            onMouseUp={() => setCropDragging(false)}
            onMouseLeave={() => setCropDragging(false)}
          >
            <img
              ref={cropImageRef}
              src={mediaUrl}
              alt={filename}
              className="max-w-full max-h-full object-contain pointer-events-none"
              draggable={false}
            />
            {/* Darken area outside crop rect */}
            {cropRect && (
              <div
                className="absolute border-2 border-white border-dashed pointer-events-none z-30"
                style={{
                  left: Math.min(cropRect.startX, cropRect.endX),
                  top: Math.min(cropRect.startY, cropRect.endY),
                  width: Math.abs(cropRect.endX - cropRect.startX),
                  height: Math.abs(cropRect.endY - cropRect.startY),
                  boxShadow: "0 0 0 9999px rgba(0,0,0,0.5)",
                }}
              />
            )}
            {/* Crop action buttons */}
            <div className="absolute bottom-4 left-1/2 -translate-x-1/2 z-40 flex gap-2">
              <button
                onClick={handleSaveCrop}
                disabled={!cropRect || Math.abs((cropRect?.endX ?? 0) - (cropRect?.startX ?? 0)) < 10}
                className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium disabled:opacity-40 hover:bg-blue-700 transition-colors"
              >
                Apply Crop
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
                onClick={() => { setShowCropEditor(false); setCropRect(null); }}
                className="px-4 py-2 bg-gray-700 text-white rounded-lg text-sm font-medium hover:bg-gray-600 transition-colors"
              >
                Cancel
              </button>
            </div>
            <p className="absolute top-4 left-1/2 -translate-x-1/2 z-40 text-white text-sm bg-black/60 px-3 py-1 rounded-full">
              Click and drag to select crop area
            </p>
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
      </div>

      {/* Bottom meta bar (shown when media is loaded) */}
      {mediaUrl && (
        <div className="px-4 py-2 bg-black/60 text-gray-400 text-xs space-y-2">
          <div className="flex items-center gap-4">
            <span className="uppercase tracking-wide font-mono">
              {mediaType === "video" ? "VIDEO" : mediaType === "gif" ? "GIF" : "PHOTO"}
            </span>
            <span className="truncate">{mimeType}</span>
            {photoIds && currentIndex >= 0 && (
              <span className="ml-auto">{currentIndex + 1} / {photoIds.length}</span>
            )}
          </div>

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
