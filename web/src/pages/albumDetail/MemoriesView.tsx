/**
 * Memories smart album — auto-generated location + date albums. Lists every
 * memory and renders the per-memory photo grid (server-resolved photos).
 */
import { useEffect, useState, useMemo } from "react";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import { api } from "../../api/client";
import { db, type CachedPhoto } from "../../db";
import AppHeader from "../../components/AppHeader";
import { GallerySkeleton } from "../../components/skeletons";
import AppIcon from "../../components/AppIcon";
import SelectablePhotoGrid from "../../components/gallery/SelectablePhotoGrid";
import useSlideshow from "../../hooks/useSlideshow";
import Slideshow from "../../components/viewer/Slideshow";
import { resolveServerPhotos } from "./resolveServerPhotos";

// ── Memories View (auto-generated location + date albums) ────────────────────

export function MemoriesView() {
  const navigate = useAppNavigate();
  const [memories, setMemories] = useState<Array<{
    id: string; name: string; city: string; country: string;
    date_label: string; photo_count: number;
    first_photo_id: string | null; first_thumb_path: string | null;
  }>>([]);
  const [loading, setLoading] = useState(true);
  const [thumbUrls, setThumbUrls] = useState<Record<string, string>>({});

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = await api.geo.listMemories();
        if (!cancelled) setMemories(data);
      } catch { /* Geo may not be enabled */ }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, []);

  // Try to load thumbnails from IDB for representative photos
  useEffect(() => {
    if (memories.length === 0) return;
    let cancelled = false;
    (async () => {
      const urls: Record<string, string> = {};
      for (const m of memories) {
        if (!m.first_photo_id) continue;
        const photo = await db.photos.where("serverPhotoId").equals(m.first_photo_id).first();
        if (cancelled) return;
        if (photo?.thumbnailData) {
          const mime = photo.thumbnailMimeType || "image/jpeg";
          urls[m.id] = URL.createObjectURL(new Blob([photo.thumbnailData], { type: mime }));
        }
      }
      if (!cancelled) setThumbUrls(urls);
    })();
    return () => {
      cancelled = true;
      Object.values(thumbUrls).forEach(URL.revokeObjectURL);
    };
  }, [memories]);

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />
      <main className="p-4">
        <div className="flex items-center gap-3 mb-4">
          <button
            onClick={() => navigate("/albums")}
            className="text-fg-muted hover:text-fg transition-colors shrink-0"
            title="Back to Albums"
          >
            <AppIcon name="back-arrow" size="w-5 h-5" />
          </button>
          <h2 className="text-xl font-semibold">Memories</h2>
        </div>

        {loading ? (
          <GallerySkeleton />
        ) : memories.length === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">No memories yet</p>
            <p className="text-fg-muted text-sm mt-1">
              Memories are auto-generated when you have 3+ photos from the same location and day
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-4">
            {memories.map((memory) => (
              <div
                key={memory.id}
                onClick={() => navigate(`/albums/smart-memories/${memory.id}`)}
                className="card card-interactive cursor-pointer overflow-hidden"
              >
                <div className="aspect-video bg-surface-raised flex items-center justify-center overflow-hidden">
                  {thumbUrls[memory.id] ? (
                    <img
                      src={thumbUrls[memory.id]}
                      alt={memory.name}
                      className="w-full h-full object-cover"
                    />
                  ) : (
                    <svg className="w-8 h-8 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M15 10.5a3 3 0 11-6 0 3 3 0 016 0z" />
                      <path strokeLinecap="round" strokeLinejoin="round" d="M19.5 10.5c0 7.142-7.5 11.25-7.5 11.25S4.5 17.642 4.5 10.5a7.5 7.5 0 1115 0z" />
                    </svg>
                  )}
                </div>
                <div className="p-3">
                  <p className="font-medium text-sm truncate">{memory.name}</p>
                  <p className="text-xs text-fg-muted">
                    {memory.photo_count} photo{memory.photo_count !== 1 ? "s" : ""} · {memory.country}
                  </p>
                </div>
              </div>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}

// ── Memory Detail View (photos from a specific memory cluster) ───────────────

export function MemoryDetailView({ memoryId }: { memoryId: string }) {
  const navigate = useAppNavigate();
  const [memoryName, setMemoryName] = useState("Memory");
  const [photos, setPhotos] = useState<CachedPhoto[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // Fetch memories list to find the name
        const memories = await api.geo.listMemories();
        const memory = memories.find(m => m.id === memoryId);
        if (!cancelled && memory) setMemoryName(memory.name);

        // Fetch photos for this memory
        const summaries = await api.geo.listMemoryPhotos(memoryId);
        const found = await resolveServerPhotos(summaries);
        if (!cancelled) setPhotos(found);
      } catch { /* memory may not exist */ }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, [memoryId]);

  // Slideshow
  const blobIds = useMemo(() => photos.map(p => p.blobId), [photos]);
  const mediaTypeMap = useMemo(() => {
    const m = new Map<string, string>();
    for (const p of photos) m.set(p.blobId, p.mediaType);
    return m;
  }, [photos]);
  const slideshow = useSlideshow(blobIds, mediaTypeMap);
  const hasPhotos = photos.some(p => p.mediaType === "photo" || p.mediaType === "gif");

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />
      <main className="p-4">
        <div className="flex items-center gap-3 mb-4">
          <button
            onClick={() => navigate("/albums/smart-memories")}
            className="text-fg-muted hover:text-fg transition-colors shrink-0"
            title="Back to Memories"
          >
            <AppIcon name="back-arrow" size="w-5 h-5" />
          </button>
          <h2 className="text-xl font-semibold truncate">{memoryName}</h2>
          <span className="text-fg-muted text-sm shrink-0">{photos.length} photos</span>
          {hasPhotos && (
            <>
            <button
              onClick={() => slideshow.start(0)}
              className="text-fg-muted hover:text-accent-600 dark:hover:text-accent-400 transition-colors shrink-0"
              title="Start Slideshow"
            >
              <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24"><path d="M8 5v14l11-7z" /></svg>
            </button>
            <button
              onClick={() => { slideshow.toggleShuffle(); slideshow.start(0); }}
              className={`transition-colors shrink-0 ${slideshow.shuffleEnabled ? "text-accent-600 dark:text-accent-400" : "text-fg-muted hover:text-accent-600 dark:hover:text-accent-400"}`}
              title={slideshow.shuffleEnabled ? "Shuffle On" : "Shuffle Off"}
            >
              <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                <path d="M10.59 9.17L5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41l-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z" />
              </svg>
            </button>
            </>
          )}
        </div>

        {loading ? (
          <GallerySkeleton />
        ) : photos.length === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">No photos found for this memory</p>
          </div>
        ) : (
          <SelectablePhotoGrid
            photos={photos}
            viewerAlbumId={`smart-memories/${memoryId}`}
            onDeleted={(ids) => setPhotos((prev) => prev.filter((p) => !ids.includes(p.blobId)))}
          />
        )}
      </main>

      {slideshow.isActive && (
        <Slideshow
          currentBlobId={slideshow.currentBlobId}
          isPlaying={slideshow.isPlaying}
          currentSlide={slideshow.currentSlide}
          totalSlides={slideshow.totalSlides}
          shuffleEnabled={slideshow.shuffleEnabled}
          intervalMs={slideshow.intervalMs}
          transition={slideshow.transition}
          direction={slideshow.direction}
          onTogglePlay={slideshow.togglePlay}
          onNext={slideshow.next}
          onPrev={slideshow.prev}
          onToggleShuffle={slideshow.toggleShuffle}
          onSetSpeed={slideshow.setSpeed}
          onSetTransition={slideshow.setTransition}
          onExit={slideshow.stop}
        />
      )}
    </div>
  );
}
