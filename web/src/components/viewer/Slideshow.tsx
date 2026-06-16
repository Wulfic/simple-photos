/**
 * Full-screen slideshow overlay.
 *
 * Displays photos sequentially or shuffled with configurable transitions
 * and auto-advance speed. Uses the existing photo loading/decryption
 * pipeline (IndexedDB cache → download + decrypt).
 *
 * Controls auto-hide after 3 seconds of inactivity and reappear on mouse move.
 */
import { useEffect, useState, useCallback, useRef } from "react";
import { api } from "../../api/client";
import { decrypt } from "../../crypto/crypto";
import { db } from "../../db";
import { base64ToUint8Array } from "../../utils/media";
import type { MediaPayload } from "../../types/media";
import SlideshowTransitions from "./SlideshowTransitions";
import type { SlideshowTransition } from "../../hooks/useSlideshow";
import { castMedia, getCastState } from "../../utils/cast";
import { useAuthStore } from "../../store/auth";
import { appendGalleryTokenParam } from "../../utils/galleryToken";

interface Props {
  currentBlobId: string | undefined;
  isPlaying: boolean;
  currentSlide: number;
  totalSlides: number;
  shuffleEnabled: boolean;
  intervalMs: number;
  transition: SlideshowTransition;
  direction: 1 | -1;
  onTogglePlay: () => void;
  onNext: () => void;
  onPrev: () => void;
  onToggleShuffle: () => void;
  onSetSpeed: (ms: number) => void;
  onSetTransition: (t: SlideshowTransition) => void;
  onExit: () => void;
}

const SPEED_OPTIONS = [
  { label: "3s", value: 3000 },
  { label: "5s", value: 5000 },
  { label: "8s", value: 8000 },
  { label: "10s", value: 10000 },
];

const TRANSITION_OPTIONS: { label: string; value: SlideshowTransition }[] = [
  { label: "Fade", value: "fade" },
  { label: "Slide", value: "slide" },
  { label: "Zoom", value: "zoom" },
  { label: "Dissolve", value: "dissolve" },
];

export default function Slideshow({
  currentBlobId,
  isPlaying,
  currentSlide,
  totalSlides,
  shuffleEnabled,
  intervalMs,
  transition,
  direction,
  onTogglePlay,
  onNext,
  onPrev,
  onToggleShuffle,
  onSetSpeed,
  onSetTransition,
  onExit,
}: Props) {
  const [mediaUrl, setMediaUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [showControls, setShowControls] = useState(true);
  const [isFullscreen, setIsFullscreen] = useState<boolean>(
    typeof document !== "undefined" && !!document.fullscreenElement,
  );
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const prevUrlRef = useRef<string | null>(null);

  // Toggle the document into / out of fullscreen. Shared between the
  // keyboard `F` shortcut and the on-bar button so the two stay in sync.
  const toggleFullscreen = useCallback(() => {
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    } else {
      document.documentElement.requestFullscreen().catch(() => {});
    }
  }, []);

  // Keep `isFullscreen` reactive to OS-level changes (Esc out of FS, etc.)
  // so the button icon flips correctly.
  useEffect(() => {
    const onChange = () => setIsFullscreen(!!document.fullscreenElement);
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);

  // ── Load current photo ─────────────────────────────────────────────────

  useEffect(() => {
    if (!currentBlobId) return;
    let cancelled = false;

    (async () => {
      setLoading(true);
      try {
        // Check IndexedDB cache first.
        const cached = await db.fullPhotos.get(currentBlobId);
        if (cached && !cancelled) {
          const blob = new Blob([cached.data], { type: cached.mimeType });
          const url = URL.createObjectURL(blob);
          if (prevUrlRef.current) URL.revokeObjectURL(prevUrlRef.current);
          prevUrlRef.current = url;
          setMediaUrl(url);
          setLoading(false);
          return;
        }

        // Resolve which server-side ID to fetch.  Edit copies and secure
        // clones share the original's storageBlobId; server-side
        // (unencrypted) photos fetch via /photos/:id/file.  Without this
        // resolution the slideshow renders a black screen for any photo
        // whose CachedPhoto.blobId differs from its server storage ID.
        const dbCached = await db.photos.get(currentBlobId).catch(() => undefined);
        if (cancelled) return;

        let bytes: ArrayBuffer;
        let mimeType: string;
        let mediaType: "photo" | "gif" | "video" | "audio";
        let filename: string;

        if (dbCached?.serverSide && dbCached.serverPhotoId) {
          // Server-side (unencrypted) — fetch the file directly. Carry the
          // access token (?token=) and, for secure-album clones, the gallery
          // unlock token so the server's secure gate is satisfied.
          const { accessToken } = useAuthStore.getState();
          const fileUrl = appendGalleryTokenParam(
            `/api/photos/${dbCached.serverPhotoId}/file${accessToken ? `?token=${encodeURIComponent(accessToken)}` : ""}`,
          );
          const res = await fetch(fileUrl, {
            credentials: "include",
          });
          if (cancelled) return;
          if (!res.ok) throw new Error(`HTTP ${res.status}`);
          bytes = await res.arrayBuffer();
          mimeType = res.headers.get("content-type") || dbCached.mimeType || "image/jpeg";
          mediaType = dbCached.mediaType;
          filename = dbCached.filename;
        } else {
          // Encrypted blob path — use storageBlobId when available so
          // copies/clones resolve to the original's server blob.
          const fetchId = dbCached?.storageBlobId || currentBlobId;
          const encrypted = await api.blobs.download(fetchId);
          if (cancelled) return;
          const decrypted = await decrypt(encrypted);
          if (cancelled) return;
          const payload: MediaPayload = JSON.parse(new TextDecoder().decode(decrypted));
          bytes = base64ToUint8Array(payload.data).buffer as ArrayBuffer;
          mimeType = payload.mime_type;
          mediaType = (payload.media_type ?? "photo") as "photo" | "gif" | "video" | "audio";
          filename = payload.filename;
        }

        const blob = new Blob([bytes], { type: mimeType });
        const url = URL.createObjectURL(blob);
        if (prevUrlRef.current) URL.revokeObjectURL(prevUrlRef.current);
        prevUrlRef.current = url;
        setMediaUrl(url);

        // Cache for future.  Skip caching for very large blobs to avoid
        // blowing the IndexedDB quota.
        if (bytes.byteLength < 50 * 1024 * 1024) {
          try {
            await db.fullPhotos.put({
              photoId: currentBlobId,
              filename,
              mimeType,
              mediaType,
              isFavorite: dbCached?.isFavorite ?? false,
              data: bytes,
              cachedAt: Date.now(),
            });
          } catch { /* non-fatal */ }
        }
      } catch {
        // Silently skip on error — slideshow will auto-advance.
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();

    return () => { cancelled = true; };
  }, [currentBlobId]);

  // Preload next photo.
  useEffect(() => {
    if (!currentBlobId) return;
    // This is a fire-and-forget preload — we don't track its result.
    const nextSlide = (currentSlide + 1) % totalSlides;
    // The hook already handles preloading via the viewer's pipeline.
  }, [currentBlobId, currentSlide, totalSlides]);

  // Cleanup on unmount.  Also exit document fullscreen so closing the
  // slideshow returns the browser to its normal windowed state — without
  // this the gallery underneath would still render in fullscreen until
  // the user pressed Esc a second time.
  useEffect(() => {
    return () => {
      if (prevUrlRef.current) URL.revokeObjectURL(prevUrlRef.current);
      if (document.fullscreenElement) {
        document.exitFullscreen().catch(() => {});
      }
    };
  }, []);

  // ── Cast: mirror the current slide to a connected Chromecast ──────────
  // When a cast session is active and the user starts a slideshow, the
  // slides should appear on the TV (replacing whatever was last sent from
  // the regular Viewer).  We resolve the local IndexedDB blobId to the
  // server's photo id so `/api/photos/:id/file` returns the correct photo.
  useEffect(() => {
    if (!currentBlobId) return;
    const { state } = getCastState();
    if (state !== "connected") return;
    let cancelled = false;
    (async () => {
      const cached = await db.photos.get(currentBlobId).catch(() => undefined);
      if (cancelled) return;
      const serverId = cached?.serverPhotoId ?? cached?.storageBlobId ?? currentBlobId;
      const { accessToken } = useAuthStore.getState();
      const castUrl = appendGalleryTokenParam(
        `${window.location.origin}/api/photos/${encodeURIComponent(serverId)}/file` +
        (accessToken ? `?token=${encodeURIComponent(accessToken)}` : ""),
      );
      const mime = cached?.mimeType ?? "image/jpeg";
      const kind: "photo" | "video" =
        cached?.mediaType === "video" || mime.startsWith("video/") ? "video" : "photo";
      // Carry the active transition so the TV replays the same effect the
      // local screen shows (videos play straight through — no transition).
      castMedia(castUrl, mime, kind, kind === "photo" ? transition : undefined);
    })();
    return () => { cancelled = true; };
  }, [currentBlobId, transition]);

  // ── Auto-hide controls ─────────────────────────────────────────────────

  const resetHideTimer = useCallback(() => {
    setShowControls(true);
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    hideTimerRef.current = setTimeout(() => {
      if (isPlaying) setShowControls(false);
    }, 3000);
  }, [isPlaying]);

  useEffect(() => {
    resetHideTimer();
    return () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    };
  }, [resetHideTimer]);

  // ── Keyboard shortcuts ─────────────────────────────────────────────────

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      switch (e.key) {
        case " ":
          e.preventDefault();
          onTogglePlay();
          break;
        case "ArrowLeft":
          onPrev();
          break;
        case "ArrowRight":
          onNext();
          break;
        case "s":
        case "S":
          onToggleShuffle();
          break;
        case "Escape":
          onExit();
          break;
        case "f":
        case "F":
          toggleFullscreen();
          break;
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onTogglePlay, onPrev, onNext, onToggleShuffle, onExit, toggleFullscreen]);

  // ── Render ─────────────────────────────────────────────────────────────

  return (
    <div
      className="fixed inset-0 z-50 bg-black select-none"
      onMouseMove={resetHideTimer}
      onClick={resetHideTimer}
    >
      {/* Photo display with transition */}
      <div className="absolute inset-0">
        {loading && !mediaUrl && (
          <div className="flex items-center justify-center h-full">
            <div className="w-8 h-8 border-2 border-white/40 border-t-white rounded-full animate-spin" />
          </div>
        )}
        {mediaUrl && (
          <SlideshowTransitions
            slideKey={currentBlobId ?? ""}
            transition={transition}
            direction={direction}
          >
            <img
              src={mediaUrl}
              className="max-w-full max-h-full object-contain"
              alt=""
              draggable={false}
            />
          </SlideshowTransitions>
        )}
      </div>

      {/* Controls bar — bottom */}
      <div
        className={`absolute bottom-0 left-0 right-0 z-10 transition-opacity duration-300 ${
          showControls ? "opacity-100" : "opacity-0 pointer-events-none"
        }`}
      >
        <div className="bg-gradient-to-t from-black/80 to-transparent pt-12 pb-4 px-4">
          {/* Progress indicator */}
          <div className="text-center text-white/70 text-xs mb-3">
            Photo {currentSlide + 1} of {totalSlides}
          </div>

          <div className="flex items-center justify-center gap-3 flex-wrap">
            {/* Previous */}
            <button
              onClick={onPrev}
              className="text-white hover:bg-white/20 w-10 h-10 rounded-full flex items-center justify-center transition-colors"
              title="Previous (Left Arrow)"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M15 19l-7-7 7-7" />
              </svg>
            </button>

            {/* Play/Pause */}
            <button
              onClick={onTogglePlay}
              className="text-white bg-white/20 hover:bg-white/30 w-12 h-12 rounded-full flex items-center justify-center transition-colors"
              title={isPlaying ? "Pause (Space)" : "Play (Space)"}
            >
              {isPlaying ? (
                <svg className="w-6 h-6" fill="currentColor" viewBox="0 0 24 24">
                  <rect x="6" y="4" width="4" height="16" rx="1" />
                  <rect x="14" y="4" width="4" height="16" rx="1" />
                </svg>
              ) : (
                <svg className="w-6 h-6" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M8 5v14l11-7z" />
                </svg>
              )}
            </button>

            {/* Next */}
            <button
              onClick={onNext}
              className="text-white hover:bg-white/20 w-10 h-10 rounded-full flex items-center justify-center transition-colors"
              title="Next (Right Arrow)"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
              </svg>
            </button>

            {/* Divider */}
            <div className="w-px h-6 bg-white/30" />

            {/* Shuffle */}
            <button
              onClick={onToggleShuffle}
              className={`w-10 h-10 rounded-full flex items-center justify-center transition-colors ${
                shuffleEnabled ? "bg-blue-600 text-white" : "text-white/70 hover:bg-white/20"
              }`}
              title={`Shuffle ${shuffleEnabled ? "On" : "Off"} (S)`}
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 4h2l3.5 7L6 18H4m14-14h2l-3.5 7L20 18h-2M9 4l3 7-3 7m6-14l-3 7 3 7" />
              </svg>
            </button>

            {/* Speed selector */}
            <div className="flex items-center gap-1">
              {SPEED_OPTIONS.map((opt) => (
                <button
                  key={opt.value}
                  onClick={() => onSetSpeed(opt.value)}
                  className={`px-2 py-1 rounded text-xs font-medium transition-colors ${
                    intervalMs === opt.value
                      ? "bg-blue-600 text-white"
                      : "text-white/70 hover:bg-white/20"
                  }`}
                >
                  {opt.label}
                </button>
              ))}
            </div>

            {/* Divider */}
            <div className="w-px h-6 bg-white/30" />

            {/* Transition selector */}
            <div className="flex items-center gap-1">
              {TRANSITION_OPTIONS.map((opt) => (
                <button
                  key={opt.value}
                  onClick={() => onSetTransition(opt.value)}
                  className={`px-2 py-1 rounded text-xs font-medium transition-colors ${
                    transition === opt.value
                      ? "bg-blue-600 text-white"
                      : "text-white/70 hover:bg-white/20"
                  }`}
                >
                  {opt.label}
                </button>
              ))}
            </div>

            {/* Divider */}
            <div className="w-px h-6 bg-white/30" />

            {/* Fullscreen toggle */}
            <button
              onClick={toggleFullscreen}
              className="text-white hover:bg-white/20 w-10 h-10 rounded-full flex items-center justify-center transition-colors"
              title={isFullscreen ? "Exit Fullscreen (F)" : "Enter Fullscreen (F)"}
              aria-label={isFullscreen ? "Exit fullscreen" : "Enter fullscreen"}
            >
              {isFullscreen ? (
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M9 9V5H5m14 0h-4v4m0 6v4h4M5 15v4h4" />
                </svg>
              ) : (
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M4 8V4h4m8 0h4v4m0 8v4h-4M8 20H4v-4" />
                </svg>
              )}
            </button>

            {/* Divider */}
            <div className="w-px h-6 bg-white/30" />

            {/* Exit */}
            <button
              onClick={onExit}
              className="text-white hover:bg-white/20 w-10 h-10 rounded-full flex items-center justify-center transition-colors"
              title="Exit Slideshow (Esc)"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
