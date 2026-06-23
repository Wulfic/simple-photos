/**
 * Pets smart album — the list of detected animal clusters and the per-pet
 * detail view (rename + photo grid).
 */
import { useEffect, useState, useMemo } from "react";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import { api } from "../../api/client";
import { db, type CachedPhoto } from "../../db";
import AppHeader from "../../components/AppHeader";
import { GallerySkeleton } from "../../components/skeletons";
import AppIcon from "../../components/AppIcon";
import SelectablePhotoGrid from "../../components/gallery/SelectablePhotoGrid";
import { useAuthStore } from "../../store/auth";
import useSlideshow from "../../hooks/useSlideshow";
import Slideshow from "../../components/viewer/Slideshow";

// ── Pets View (smart album for detected animal clusters) ─────────────────────

export function PetsView() {
  const navigate = useAppNavigate();
  const [clusters, setClusters] = useState<Array<{
    id: number; label: string | null; species: string; photo_count: number;
    representative: string | null;
  }>>([]);
  const [loading, setLoading] = useState(true);
  const [thumbUrls, setThumbUrls] = useState<Record<number, string>>({});
  const { accessToken } = useAuthStore();

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = await api.ai.listPetClusters();
        if (!cancelled) setClusters(data);
      } catch { /* AI may not be enabled */ }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    if (clusters.length === 0) return;
    let cancelled = false;
    (async () => {
      const urls: Record<number, string> = {};
      for (const c of clusters) {
        if (!c.representative) continue;
        const photo = await db.photos.where("serverPhotoId").equals(c.representative).first();
        if (cancelled) return;
        if (photo?.thumbnailData) {
          const mime = photo.thumbnailMimeType || "image/jpeg";
          urls[c.id] = URL.createObjectURL(new Blob([photo.thumbnailData], { type: mime }));
        } else {
          urls[c.id] = accessToken
            ? `${api.photos.thumbUrl(c.representative)}?token=${accessToken}`
            : api.photos.thumbUrl(c.representative);
        }
      }
      if (!cancelled) setThumbUrls(urls);
    })();
    return () => {
      cancelled = true;
      Object.values(thumbUrls).forEach(URL.revokeObjectURL);
    };
  }, [clusters]);

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
          <h2 className="text-xl font-semibold">Pets</h2>
        </div>

        {loading ? (
          <GallerySkeleton />
        ) : clusters.length === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">No pets detected yet</p>
            <p className="text-fg-muted text-sm mt-1">
              Enable AI processing in Settings to detect pets in your photos
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4">
            {clusters.map((cluster) => (
              <div
                key={cluster.id}
                onClick={() => navigate(`/albums/smart-pets/${cluster.id}`)}
                className="card card-interactive p-3 cursor-pointer"
              >
                <div className="aspect-square bg-surface-raised rounded-full mb-2 mx-auto w-24 h-24 flex items-center justify-center overflow-hidden">
                  {thumbUrls[cluster.id] ? (
                    <img
                      src={thumbUrls[cluster.id]}
                      alt={cluster.label || cluster.species}
                      className="w-full h-full object-cover rounded-full"
                    />
                  ) : (
                    <svg className="w-10 h-10 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M12 6.75a.75.75 0 110-1.5.75.75 0 010 1.5zM12 12.75a.75.75 0 110-1.5.75.75 0 010 1.5zM12 18.75a.75.75 0 110-1.5.75.75 0 010 1.5z" />
                    </svg>
                  )}
                </div>
                <p className="font-medium text-center text-sm truncate capitalize">
                  {cluster.label || `Unknown ${cluster.species}`}
                </p>
                <p className="text-xs text-fg-muted text-center">
                  {cluster.photo_count} photo{cluster.photo_count !== 1 ? "s" : ""}
                </p>
              </div>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}

export function PetDetailView({ clusterId }: { clusterId: number }) {
  const navigate = useAppNavigate();
  const [clusterName, setClusterName] = useState<string>("Pet");
  const [species, setSpecies] = useState<string>("");
  const [photos, setPhotos] = useState<CachedPhoto[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);
  const [nameInput, setNameInput] = useState("");

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const clusters = await api.ai.listPetClusters();
        const cluster = clusters.find(c => c.id === clusterId);
        if (!cancelled && cluster) {
          setClusterName(cluster.label || `Unknown ${cluster.species}`);
          setSpecies(cluster.species);
          setNameInput(cluster.label || "");
        }

        const detections = await api.ai.listPetClusterPhotos(clusterId);
        const photoIds = [...new Set(detections.map(d => d.photo_id))];

        const found: CachedPhoto[] = [];
        for (const pid of photoIds) {
          const photo = await db.photos.where("serverPhotoId").equals(pid).first();
          if (photo) found.push(photo);
        }
        if (!cancelled) setPhotos(found);
      } catch { /* cluster may not exist */ }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, [clusterId]);

  async function saveName() {
    try {
      await api.ai.renamePetCluster(clusterId, nameInput.trim());
      setClusterName(nameInput.trim() || `Unknown ${species}`);
      setEditing(false);
    } catch { /* ignore */ }
  }

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
            onClick={() => navigate("/albums/smart-pets")}
            className="text-fg-muted hover:text-fg transition-colors shrink-0"
            title="Back to Pets"
          >
            <AppIcon name="back-arrow" size="w-5 h-5" />
          </button>
          {editing ? (
            <form onSubmit={(e) => { e.preventDefault(); saveName(); }} className="flex items-center gap-2">
              <input
                type="text"
                value={nameInput}
                onChange={(e) => setNameInput(e.target.value)}
                className="input w-auto text-lg font-semibold py-1"
                autoFocus
                maxLength={100}
              />
              <button type="submit" className="text-accent-600 text-sm font-medium">Save</button>
              <button type="button" onClick={() => setEditing(false)} className="text-fg-muted text-sm">Cancel</button>
            </form>
          ) : (
            <>
              <h2 className="text-xl font-semibold truncate capitalize">{clusterName}</h2>
              <button
                onClick={() => setEditing(true)}
                className="text-fg-muted hover:text-fg text-sm"
                title="Rename"
              >
                ✏️
              </button>
            </>
          )}
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
            <p className="text-fg-muted">No photos found for this pet</p>
          </div>
        ) : (
          <SelectablePhotoGrid
            photos={photos}
            viewerAlbumId={`smart-pets/${clusterId}`}
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
