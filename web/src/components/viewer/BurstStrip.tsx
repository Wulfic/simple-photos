/**
 * BurstStrip — horizontal filmstrip for browsing burst photo frames.
 *
 * Shown at the bottom of the Viewer when viewing a burst photo.
 * Loads all frames in the burst group via the burst API endpoint
 * and displays thumbnails. Tapping a frame navigates to it.
 */
import { useEffect, useState, useRef } from "react";
import { api } from "../../api/client";
import { db } from "../../db";
import { decrypt } from "../../crypto/crypto";

interface BurstFrame {
  id: string;
  filename: string;
  thumbUrl: string | null;
}

/** A secure-gallery item sharing the current photo's burst_id (see Viewer.tsx). */
interface SecureBurstFrame {
  id: string;
  blob_id: string;
  encrypted_thumb_blob_id?: string | null;
}

interface BurstStripProps {
  burstId: string;
  currentPhotoId: string;
  onSelectFrame: (photoId: string) => void;
  visible: boolean;
  /**
   * When opened from a secure gallery, the regular burst-frames API can't be
   * used — its frames are the original (hidden) photos, not the secure
   * gallery's own encrypted clones. Pass the already-fetched secure items
   * sharing this burst_id instead, and frames are resolved from those
   * (encrypted thumb blob, decrypted client-side) rather than fetched here.
   */
  secureFrames?: SecureBurstFrame[];
}

// Tile width (w-12) + gap (gap-1.5) in px — used to size the visible window
// to ~5 frames (matching the Android BurstStripOverlay) and to compute arrow
// scroll steps.
const TILE_STEP = 48 + 6;
const VISIBLE_FRAMES = 5;

export default function BurstStrip({ burstId, currentPhotoId, onSelectFrame, visible, secureFrames }: BurstStripProps) {
  const [frames, setFrames] = useState<BurstFrame[]>([]);
  const [loading, setLoading] = useState(true);
  const [canScrollLeft, setCanScrollLeft] = useState(false);
  const [canScrollRight, setCanScrollRight] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setFrames([]);

    // Object URLs created for IDB-cached thumbnails — must be revoked when
    // the strip unmounts or switches bursts, or every viewed burst leaks
    // its decoded thumbnails for the lifetime of the tab.
    const createdUrls: string[] = [];

    // Patch a single frame's thumbnail into state once it resolves, matched by id.
    const patchThumb = (frameId: string, thumbUrl: string) => {
      if (cancelled) return;
      createdUrls.push(thumbUrl);
      setFrames((prev) => prev.map((f) => (f.id === frameId ? { ...f, thumbUrl } : f)));
    };

    (async () => {
      try {
        if (secureFrames) {
          // Secure-gallery mode — frames are already known (passed in). Show
          // the strip IMMEDIATELY with placeholder tiles, then resolve each
          // thumbnail (IDB cache → encrypted-thumb blob decrypt) in PARALLEL,
          // patching them in as they arrive. Previously every frame was
          // decrypted sequentially before the strip rendered at all, so a large
          // burst (dozens of frames) made the strip look like it never appeared.
          if (!cancelled) {
            setFrames(secureFrames.map((sf) => ({ id: sf.blob_id, filename: sf.blob_id, thumbUrl: null })));
            setLoading(false);
          }
          await Promise.all(
            secureFrames.map(async (sf) => {
              try {
                const cached = await db.photos.get(sf.blob_id);
                if (cached?.thumbnailData) {
                  const mime = cached.thumbnailMimeType || "image/jpeg";
                  patchThumb(sf.blob_id, URL.createObjectURL(new Blob([cached.thumbnailData], { type: mime })));
                  return;
                }
                if (sf.encrypted_thumb_blob_id) {
                  const encData = await api.blobs.download(sf.encrypted_thumb_blob_id);
                  const plaintext = await decrypt(encData);
                  const json = JSON.parse(new TextDecoder().decode(plaintext));
                  const b64 = json.data as string;
                  if (b64) {
                    const binary = atob(b64);
                    const bytes = new Uint8Array(binary.length);
                    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
                    patchThumb(sf.blob_id, URL.createObjectURL(new Blob([bytes], { type: json.mime_type || "image/jpeg" })));
                  }
                }
              } catch (e) {
                console.warn("[BurstStrip] failed to load secure thumb:", e); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
              }
            }),
          );
        } else {
          const burstPhotos = await api.photos.burstFrames(burstId);
          if (cancelled) return;

          // Show frames immediately, then resolve thumbnails (IDB cache → server
          // thumb endpoint) in parallel and patch them in.
          setFrames(burstPhotos.map((bp) => ({ id: bp.id, filename: bp.filename, thumbUrl: null })));
          setLoading(false);
          await Promise.all(
            burstPhotos.map(async (bp) => {
              const cached = await db.photos.get(bp.id);
              if (cached?.thumbnailData) {
                const mime = cached.thumbnailMimeType || "image/jpeg";
                patchThumb(bp.id, URL.createObjectURL(new Blob([cached.thumbnailData], { type: mime })));
              } else if (bp.thumb_path) {
                // Server thumbnail endpoint — a plain URL, no object URL to revoke.
                if (!cancelled) setFrames((prev) => prev.map((f) => (f.id === bp.id ? { ...f, thumbUrl: api.photos.thumbUrl(bp.id) } : f)));
              }
            }),
          );
        }
      } catch (e) {
        console.error("[BurstStrip] failed to load burst frames:", e);
        // Strip stays hidden (frames.length <= 1) — viewer remains usable.
      }
      if (!cancelled) setLoading(false);
    })();

    return () => {
      cancelled = true;
      for (const url of createdUrls) URL.revokeObjectURL(url);
    };
  }, [burstId, secureFrames]);

  // Scroll active frame into view
  useEffect(() => {
    if (!scrollRef.current) return;
    const activeEl = scrollRef.current.querySelector(`[data-frame-id="${currentPhotoId}"]`);
    activeEl?.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "center" });
  }, [currentPhotoId, frames]);

  // Translate vertical mouse-wheel into horizontal scroll. The scrollbar is
  // hidden, and on a desktop with only a vertical wheel a large burst (e.g. 46
  // frames) was un-scrollable. A JSX onWheel isn't enough: React registers
  // wheel as a *passive* listener (so preventDefault is a no-op) and the
  // viewer's own wheel/zoom handler competes. Attach a NON-passive native
  // listener so we can preventDefault and own the gesture over the strip.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      // Honour horizontal-capable input (trackpads) directly; otherwise map
      // the dominant vertical delta onto the horizontal axis.
      let delta = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
      if (delta === 0) return;
      if (el.scrollWidth <= el.clientWidth) return; // nothing to scroll
      // Normalise delta to pixels. Many desktop mice report deltaMode=1
      // (lines, deltaY≈±3) rather than pixels (±100): scrolling by 3px/notch
      // felt like "it doesn't scroll". Convert lines→px and pages→px, then
      // apply a small multiplier so one notch advances ~a frame or two.
      if (e.deltaMode === 1) delta *= 16;
      else if (e.deltaMode === 2) delta *= el.clientWidth;
      e.preventDefault();
      e.stopPropagation();
      el.scrollLeft += delta * 1.5;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [frames]);

  // Track whether there's more to scroll on either side, so the arrow
  // buttons can dim/disable instead of overscrolling past the ends.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const update = () => {
      setCanScrollLeft(el.scrollLeft > 1);
      setCanScrollRight(el.scrollLeft < el.scrollWidth - el.clientWidth - 1);
    };
    update();
    el.addEventListener("scroll", update);
    window.addEventListener("resize", update);
    return () => {
      el.removeEventListener("scroll", update);
      window.removeEventListener("resize", update);
    };
  }, [frames]);

  const scrollByFrames = (count: number) => {
    scrollRef.current?.scrollBy({ left: count * TILE_STEP, behavior: "smooth" });
  };

  if (!visible || frames.length <= 1) return null;

  const hasOverflow = frames.length > VISIBLE_FRAMES;

  return (
    <div className="absolute bottom-20 left-0 right-0 z-30 flex justify-center pointer-events-none">
      <div className="relative pointer-events-auto">
        {hasOverflow && (
          <button
            onClick={() => scrollByFrames(-3)}
            disabled={!canScrollLeft}
            className={`absolute -left-3 top-1/2 -translate-y-1/2 z-10 w-7 h-7 flex items-center justify-center rounded-full bg-black/70 text-white transition-opacity ${
              canScrollLeft ? "opacity-100 hover:bg-black/90" : "opacity-0 pointer-events-none"
            }`}
            aria-label="Scroll burst frames left"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 19.5L8.25 12l7.5-7.5" />
            </svg>
          </button>
        )}
        <div
          ref={scrollRef}
          className="flex gap-1.5 px-3 py-2 bg-black/70 rounded-xl backdrop-blur-sm overflow-x-auto scrollbar-hide"
          style={{ scrollbarWidth: "none", maxWidth: `min(90vw, ${VISIBLE_FRAMES * TILE_STEP + 24}px)` }}
        >
          {loading ? (
            <div className="flex items-center gap-2 px-4 py-1">
              <div className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
              <span className="text-white/60 text-xs">Loading burst…</span>
            </div>
          ) : (
            frames.map((frame, idx) => {
              const isActive = frame.id === currentPhotoId;
              return (
                <button
                  key={frame.id}
                  data-frame-id={frame.id}
                  onClick={() => onSelectFrame(frame.id)}
                  className={`flex-shrink-0 w-12 h-12 rounded-lg overflow-hidden border-2 transition-all ${
                    isActive ? "border-white scale-110" : "border-transparent opacity-70 hover:opacity-100"
                  }`}
                  title={frame.filename}
                >
                  {frame.thumbUrl ? (
                    <img src={frame.thumbUrl} alt={`Frame ${idx + 1}`} className="w-full h-full object-cover" />
                  ) : (
                    <div className="w-full h-full bg-gray-600 flex items-center justify-center text-white text-[10px]">
                      {idx + 1}
                    </div>
                  )}
                </button>
              );
            })
          )}
        </div>
        {hasOverflow && (
          <button
            onClick={() => scrollByFrames(3)}
            disabled={!canScrollRight}
            className={`absolute -right-3 top-1/2 -translate-y-1/2 z-10 w-7 h-7 flex items-center justify-center rounded-full bg-black/70 text-white transition-opacity ${
              canScrollRight ? "opacity-100 hover:bg-black/90" : "opacity-0 pointer-events-none"
            }`}
            aria-label="Scroll burst frames right"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M8.25 4.5l7.5 7.5-7.5 7.5" />
            </svg>
          </button>
        )}
      </div>
    </div>
  );
}
