/**
 * Memories smart album — auto-generated location + date albums. Lists every
 * memory and renders the per-memory photo grid (server-resolved photos).
 */
import { useEffect, useState } from "react";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import { api } from "../../api/client";
import { type CachedPhoto } from "../../db";
import AppHeader from "../../components/AppHeader";
import { GallerySkeleton } from "../../components/skeletons";
import AppIcon from "../../components/AppIcon";
import SelectablePhotoGrid from "../../components/gallery/SelectablePhotoGrid";
import { useIdbThumbnailMap } from "../../hooks/useIdbThumbnailMap";
import { usePhotoSlideshow } from "../../hooks/useSlideshow";
import SlideshowHost from "../../components/viewer/SlideshowHost";
import SlideshowTriggers from "../../components/viewer/SlideshowTriggers";
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
  const thumbUrls = useIdbThumbnailMap(
    memories.map((m) => ({ key: m.id, photoId: m.first_photo_id })),
  );

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

  const slideshow = usePhotoSlideshow(photos);

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
          <SlideshowTriggers slideshow={slideshow} />
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

      <SlideshowHost slideshow={slideshow} />
    </div>
  );
}
