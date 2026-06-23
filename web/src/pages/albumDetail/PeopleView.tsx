/**
 * People smart album — the list of detected face clusters and the per-person
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
import { useIdbThumbnailMap } from "../../hooks/useIdbThumbnailMap";
import { usePhotoSlideshow } from "../../hooks/useSlideshow";
import SlideshowHost from "../../components/viewer/SlideshowHost";
import SlideshowTriggers from "../../components/viewer/SlideshowTriggers";

// ── People View (Face Clusters) ──────────────────────────────────────────────

export function PeopleView() {
  const navigate = useAppNavigate();
  const [clusters, setClusters] = useState<Array<{
    id: number; label: string | null; photo_count: number;
    representative: string | null;
  }>>([]);
  const [loading, setLoading] = useState(true);
  const thumbUrls = useIdbThumbnailMap(
    clusters.map((c) => ({ key: c.id, photoId: c.representative })),
  );

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = await api.ai.listFaceClusters();
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
          <h2 className="text-xl font-semibold">People</h2>
        </div>

        {loading ? (
          <GallerySkeleton />
        ) : clusters.length === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">No faces detected yet</p>
            <p className="text-fg-muted text-sm mt-1">
              Enable AI processing in Settings to detect faces
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4">
            {clusters.map((cluster) => (
              <div
                key={cluster.id}
                onClick={() => navigate(`/albums/smart-people/${cluster.id}`)}
                className="card card-interactive p-3 cursor-pointer"
              >
                <div className="aspect-square bg-surface-raised rounded-full mb-2 mx-auto w-24 h-24 flex items-center justify-center overflow-hidden">
                  {thumbUrls[cluster.id] ? (
                    <img
                      src={thumbUrls[cluster.id]}
                      alt={cluster.label || "Unknown"}
                      className="w-full h-full object-cover rounded-full"
                    />
                  ) : (
                    <svg className="w-10 h-10 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z" />
                    </svg>
                  )}
                </div>
                <p className="font-medium text-center text-sm truncate">
                  {cluster.label || "Unknown Person"}
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

// ── Person Detail View (photos of a specific face cluster) ───────────────────

export function PersonDetailView({ clusterId }: { clusterId: number }) {
  const navigate = useAppNavigate();
  const [clusterName, setClusterName] = useState<string>("Person");
  const [photos, setPhotos] = useState<CachedPhoto[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);
  const [nameInput, setNameInput] = useState("");

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // Fetch cluster info for the label
        const clusters = await api.ai.listFaceClusters();
        const cluster = clusters.find(c => c.id === clusterId);
        if (!cancelled && cluster) {
          setClusterName(cluster.label || "Unknown Person");
          setNameInput(cluster.label || "");
        }

        // Fetch face detections to get photo IDs
        const detections = await api.ai.listClusterPhotos(clusterId);
        const photoIds = [...new Set(detections.map(d => d.photo_id))];

        // Look up photos in IDB by serverPhotoId
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
      await api.ai.renameFaceCluster(clusterId, nameInput.trim());
      setClusterName(nameInput.trim() || "Unknown Person");
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
            onClick={() => navigate("/albums/smart-people")}
            className="text-fg-muted hover:text-fg transition-colors shrink-0"
            title="Back to People"
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
              <h2 className="text-xl font-semibold truncate">{clusterName}</h2>
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
            <p className="text-fg-muted">No photos found for this person</p>
          </div>
        ) : (
          <SelectablePhotoGrid
            photos={photos}
            viewerAlbumId={`smart-people/${clusterId}`}
            onDeleted={(ids) => setPhotos((prev) => prev.filter((p) => !ids.includes(p.blobId)))}
          />
        )}
      </main>

      <SlideshowHost slideshow={slideshow} />
    </div>
  );
}
