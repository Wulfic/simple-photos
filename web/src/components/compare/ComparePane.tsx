/**
 * A single media pane in the split-screen Compare view (#21).
 *
 * Self-contained: loads + decrypts one photo/gif/video by blobId (its own
 * preload cache, so panes never share state) and provides INDEPENDENT
 * zoom/pan for photos & GIFs — wheel + double-click zoom, drag-to-pan when
 * zoomed, and two-finger pinch on touch. Videos render with native controls
 * (looping) and are not zoomable.
 */
import { useEffect, useRef, useState, useCallback } from "react";
import { db } from "../../db";
import { resolveThumb } from "../../db/thumbs";
import useViewerMedia from "../../hooks/useViewerMedia";
import type { PreloadEntry } from "../../types/media";
import { diagnosticLogger } from "../../utils/diagnosticLogger";

const MAX_SCALE = 5;

interface ComparePaneProps {
  /** IndexedDB blobId of the photo to show in this pane. */
  photoId: string;
  /** Small badge shown top-left ("1" / "2") to disambiguate the two panes. */
  badge?: string;
}

export default function ComparePane({ photoId, badge }: ComparePaneProps) {
  // Per-pane cache — the two panes must not share media state.
  const preloadCache = useRef<Map<string, PreloadEntry>>(new Map());
  const {
    mediaUrl, setMediaUrl,
    previewUrl, setPreviewUrl,
    filename,
    mimeType,
    mediaType,
    loading, setLoading,
    error, setError,
    videoError, setVideoError,
    loadEncryptedMedia,
  } = useViewerMedia(preloadCache);

  const containerRef = useRef<HTMLDivElement>(null);

  // ── Independent zoom/pan state ─────────────────────────────────────────
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  // Active pointers for pan / pinch. Keyed by pointerId.
  const pointers = useRef<Map<number, { x: number; y: number }>>(new Map());
  const pinchStart = useRef<{ dist: number; scale: number } | null>(null);

  const resetZoom = useCallback(() => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }, []);

  const isZoomable = mediaType === "photo" || mediaType === "gif";

  // ── Load media whenever the target photo changes ───────────────────────
  useEffect(() => {
    if (!photoId) return;
    let cancelled = false;
    resetZoom();
    setLoading(true);
    setError("");
    setVideoError(false);
    setMediaUrl(null);
    (async () => {
      const cached = await db.photos.get(photoId).catch(() => undefined);
      if (cancelled) return;
      const thumb = await resolveThumb(cached);
      if (cancelled) return;
      if (thumb) {
        setPreviewUrl(URL.createObjectURL(new Blob([thumb.data], { type: thumb.mime })));
      }
      // Copies reference the original's server blob via storageBlobId.
      const fetchId = cached?.storageBlobId || photoId;
      diagnosticLogger.debug("COMPARE", `Loading pane media fetchId=${fetchId}`);
      loadEncryptedMedia(fetchId);
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [photoId]);

  // ── Revoke every object URL this pane created when it unmounts ──────────
  useEffect(() => {
    const cache = preloadCache.current;
    return () => {
      for (const entry of cache.values()) {
        try { URL.revokeObjectURL(entry.url); } catch { /* ignore */ }
      }
      cache.clear();
    };
  }, []);

  // ── Gesture handlers (pointer events unify mouse + touch) ───────────────
  const clampScale = (s: number) => Math.min(Math.max(s, 1), MAX_SCALE);

  const onWheel = useCallback((e: React.WheelEvent) => {
    if (!isZoomable) return;
    e.preventDefault();
    setScale((prev) => {
      const next = clampScale(prev - e.deltaY * 0.002);
      if (next <= 1) setOffset({ x: 0, y: 0 });
      return next;
    });
  }, [isZoomable]);

  const onDoubleClick = useCallback(() => {
    if (!isZoomable) return;
    if (scale > 1) resetZoom();
    else setScale(2);
  }, [isZoomable, scale, resetZoom]);

  const onPointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!isZoomable) return;
    e.currentTarget.setPointerCapture?.(e.pointerId);
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.current.size === 2) {
      const [a, b] = Array.from(pointers.current.values());
      pinchStart.current = { dist: Math.hypot(a.x - b.x, a.y - b.y), scale };
    }
  }, [isZoomable, scale]);

  const onPointerMove = useCallback((e: React.PointerEvent) => {
    if (!isZoomable) return;
    const prev = pointers.current.get(e.pointerId);
    if (!prev) return;
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });

    if (pointers.current.size >= 2 && pinchStart.current) {
      const [a, b] = Array.from(pointers.current.values());
      const dist = Math.hypot(a.x - b.x, a.y - b.y);
      const ratio = pinchStart.current.dist > 0 ? dist / pinchStart.current.dist : 1;
      setScale(() => {
        const next = clampScale(pinchStart.current!.scale * ratio);
        if (next <= 1) setOffset({ x: 0, y: 0 });
        return next;
      });
    } else if (scale > 1) {
      // Single-pointer drag → pan the zoomed image.
      setOffset((o) => ({ x: o.x + (e.clientX - prev.x), y: o.y + (e.clientY - prev.y) }));
    }
  }, [isZoomable, scale]);

  const onPointerUp = useCallback((e: React.PointerEvent) => {
    pointers.current.delete(e.pointerId);
    if (pointers.current.size < 2) pinchStart.current = null;
  }, []);

  const transform = scale > 1
    ? `translate(${offset.x}px, ${offset.y}px) scale(${scale})`
    : undefined;

  return (
    <div
      ref={containerRef}
      className="relative w-full h-full overflow-hidden bg-black flex items-center justify-center select-none touch-none"
      onWheel={onWheel}
      onDoubleClick={onDoubleClick}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      {/* Badge */}
      {badge && (
        <div className="absolute top-2 left-2 z-20 w-6 h-6 flex items-center justify-center rounded-full bg-black/60 text-white text-xs font-semibold pointer-events-none">
          {badge}
        </div>
      )}

      {/* Blurred thumbnail while full media loads */}
      {previewUrl && loading && (
        <img src={previewUrl} alt="" className="absolute inset-0 w-full h-full object-contain blur-sm opacity-60 pointer-events-none" />
      )}
      {loading && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
          <div className="w-7 h-7 border-2 border-white/30 border-t-white rounded-full animate-spin" />
        </div>
      )}
      {error && <p className="text-red-400 text-sm px-4 text-center z-10">{error}</p>}

      {/* Photo / GIF */}
      {mediaUrl && isZoomable && (
        <img
          src={mediaUrl}
          alt={filename}
          className="w-full h-full object-contain"
          draggable={false}
          style={{
            transform,
            cursor: scale > 1 ? "grab" : "default",
            transition: scale > 1 ? "none" : "transform 150ms",
          }}
          onError={() => {
            diagnosticLogger.error("COMPARE", `Pane image failed to render: ${filename} (mime=${mimeType})`);
            setError(`This image could not be displayed (${mimeType || "unknown format"}).`);
          }}
        />
      )}

      {/* Video — native controls, looping, muted autoplay (two panes can both
          be video; muted avoids double audio — unmute via the pane controls). */}
      {mediaUrl && mediaType === "video" && !videoError && (
        <video
          src={mediaUrl}
          playsInline
          controls
          loop
          muted
          autoPlay
          className="w-full h-full object-contain"
          style={{ background: "black" }}
          onError={() => setVideoError(true)}
        />
      )}
      {mediaUrl && mediaType === "video" && videoError && (
        <p className="text-gray-400 text-xs px-4 text-center z-10">This video format can’t be played here.</p>
      )}

      {/* Audio (rare in compare, but handle gracefully) */}
      {mediaUrl && mediaType === "audio" && (
        <div className="flex flex-col items-center justify-center text-center px-4">
          <div className="text-gray-400 text-5xl mb-4">♫</div>
          <p className="text-gray-300 text-xs mb-4 truncate max-w-[80%]">{filename}</p>
          <audio src={mediaUrl} controls className="w-full max-w-xs" />
        </div>
      )}
    </div>
  );
}
