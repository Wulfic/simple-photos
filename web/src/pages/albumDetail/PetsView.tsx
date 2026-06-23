/**
 * Pets smart album — the list of detected animal clusters and the per-pet
 * detail view (rename + photo grid).
 */
import { useEffect, useState } from "react";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import { api } from "../../api/client";
import { db, type CachedPhoto } from "../../db";
import AppHeader from "../../components/AppHeader";
import { GallerySkeleton } from "../../components/skeletons";
import AppIcon from "../../components/AppIcon";
import SelectablePhotoGrid from "../../components/gallery/SelectablePhotoGrid";
import { useAuthStore } from "../../store/auth";
import { useIdbThumbnailMap } from "../../hooks/useIdbThumbnailMap";
import { usePhotoSlideshow } from "../../hooks/useSlideshow";
import SlideshowHost from "../../components/viewer/SlideshowHost";
import SlideshowTriggers from "../../components/viewer/SlideshowTriggers";

// ── Pets View (smart album for detected animal clusters) ─────────────────────

export function PetsView() {
  const navigate = useAppNavigate();
  const [clusters, setClusters] = useState<Array<{
    id: number; label: string | null; species: string; photo_count: number;
    representative: string | null;
  }>>([]);
  const [loading, setLoading] = useState(true);
  const { accessToken } = useAuthStore();
  const thumbUrls = useIdbThumbnailMap(
    clusters.map((c) => ({ key: c.id, photoId: c.representative })),
    {
      fallbackUrl: (id) =>
        accessToken ? `${api.photos.thumbUrl(id)}?token=${accessToken}` : api.photos.thumbUrl(id),
    },
  );

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

  const slideshow = usePhotoSlideshow(photos);

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
          <SlideshowTriggers slideshow={slideshow} />
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

      <SlideshowHost slideshow={slideshow} />
    </div>
  );
}
