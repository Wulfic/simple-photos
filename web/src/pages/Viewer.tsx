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

  const videoRef = useRef<HTMLVideoElement>(null);

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
        <div className="flex gap-3">
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
        {mediaUrl && (mediaType === "photo" || mediaType === "gif") && (
          <img
            src={mediaUrl}
            alt={filename}
            className="max-w-full max-h-full object-contain"
            style={{ imageRendering: mediaType === "gif" ? "auto" : undefined }}
          />
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
        <div className="px-4 py-2 bg-black/60 text-gray-400 text-xs flex items-center gap-4">
          <span className="uppercase tracking-wide font-mono">
            {mediaType === "video" ? "VIDEO" : mediaType === "gif" ? "GIF" : "PHOTO"}
          </span>
          <span className="truncate">{mimeType}</span>
          {photoIds && currentIndex >= 0 && (
            <span className="ml-auto">{currentIndex + 1} / {photoIds.length}</span>
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
