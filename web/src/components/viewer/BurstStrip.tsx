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

interface BurstFrame {
  id: string;
  filename: string;
  thumbUrl: string | null;
}

interface BurstStripProps {
  burstId: string;
  currentPhotoId: string;
  onSelectFrame: (photoId: string) => void;
  visible: boolean;
}

export default function BurstStrip({ burstId, currentPhotoId, onSelectFrame, visible }: BurstStripProps) {
  const [frames, setFrames] = useState<BurstFrame[]>([]);
  const [loading, setLoading] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setFrames([]);

    (async () => {
      try {
        const burstPhotos = await api.photos.burstFrames(burstId);
        if (cancelled) return;

        // Load thumbnails for each frame from IDB cache or server thumb endpoint
        const loadedFrames: BurstFrame[] = [];
        for (const bp of burstPhotos) {
          let thumbUrl: string | null = null;

          // Check IDB first (photo may be cached locally)
          const cached = await db.photos.get(bp.id);
          if (cached?.thumbnailData) {
            const mime = cached.thumbnailMimeType || "image/jpeg";
            thumbUrl = URL.createObjectURL(new Blob([cached.thumbnailData], { type: mime }));
          } else if (bp.thumb_path) {
            // Use server thumbnail endpoint
            thumbUrl = api.photos.thumbUrl(bp.id);
          }

          loadedFrames.push({ id: bp.id, filename: bp.filename, thumbUrl });
        }

        if (!cancelled) setFrames(loadedFrames);
      } catch { /* API error — hide strip */ }
      if (!cancelled) setLoading(false);
    })();

    return () => { cancelled = true; };
  }, [burstId]);

  // Scroll active frame into view
  useEffect(() => {
    if (!scrollRef.current) return;
    const activeEl = scrollRef.current.querySelector(`[data-frame-id="${currentPhotoId}"]`);
    activeEl?.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "center" });
  }, [currentPhotoId, frames]);

  if (!visible || frames.length <= 1) return null;

  return (
    <div className="absolute bottom-20 left-0 right-0 z-30 flex justify-center pointer-events-none">
      <div
        ref={scrollRef}
        className="flex gap-1.5 px-3 py-2 bg-black/70 rounded-xl backdrop-blur-sm overflow-x-auto max-w-[90vw] pointer-events-auto scrollbar-hide"
        style={{ scrollbarWidth: "none" }}
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
    </div>
  );
}
