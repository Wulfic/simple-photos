/**
 * Smart album view — renders the synthetic albums (Recently Added, Favorites,
 * Photos, GIFs, Videos, Audio) that are computed from the local encrypted
 * photo cache rather than a user-created album manifest.
 */
import { useEffect, useState, useMemo, useRef } from "react";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import { api } from "../../api/client";
import { db, type CachedPhoto } from "../../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../../components/AppHeader";
import { GallerySkeleton } from "../../components/skeletons";
import AppIcon from "../../components/AppIcon";
import SelectablePhotoGrid from "../../components/gallery/SelectablePhotoGrid";
import { usePhotoSlideshow } from "../../hooks/useSlideshow";
import SlideshowHost from "../../components/viewer/SlideshowHost";
import SlideshowTriggers from "../../components/viewer/SlideshowTriggers";

// ── Smart album definitions ───────────────────────────────────────────────────

type SmartAlbumDef = {
  label: string;
  filterEncrypted: (p: CachedPhoto) => boolean;
  /** When set, override the default takenAt-desc ordering. "addedAt" sorts by
   *  library import order (falls back to takenAt when addedAt is absent). */
  sortBy?: "addedAt";
  /** When set, cap the album to the N most-recent items after sorting. */
  limit?: number;
};

const SMART_ALBUM_DEFS: Record<string, SmartAlbumDef> = {
  "smart-recent": {
    label: "Recently Added",
    filterEncrypted: () => true,
    sortBy: "addedAt",
    limit: 100,
  },
  "smart-favorites": {
    label: "Favorites",
    filterEncrypted: (p) => !!p.isFavorite,
  },
  "smart-photos": {
    label: "Photos",
    filterEncrypted: (p) => p.mediaType === "photo" || p.mediaType === "gif",
  },
  "smart-gifs": {
    label: "GIFs",
    filterEncrypted: (p) => p.mediaType === "gif",
  },
  "smart-videos": {
    label: "Videos",
    filterEncrypted: (p) => p.mediaType === "video",
  },
  "smart-audio": {
    label: "Audio",
    filterEncrypted: (p) => p.mediaType === "audio",
  },
};

export function isSmartAlbum(id: string | undefined): id is string {
  return !!id && id in SMART_ALBUM_DEFS;
}

export default function SmartAlbumView({ albumId }: { albumId: string }) {
  const navigate = useAppNavigate();
  const def = SMART_ALBUM_DEFS[albumId];
  const [loading, setLoading] = useState(true);
  const [secureBlobIds, setSecureBlobIds] = useState<Set<string>>(new Set());

  // Encrypted photos from IndexedDB — only read blobId + mediaType + takenAt
  // to minimise re-render cost.
  const allEncryptedPhotos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

  useEffect(() => {
    (async () => {
      try {
        const secureRes = await api.secureGalleries.secureBlobIds();
        setSecureBlobIds(new Set(secureRes.blob_ids));
      } catch { /* secure galleries may not be available */ }
      setLoading(false);
    })();
  }, []);

  // Compute filtered photos.  Stabilise so that identical blob ID lists
  // don't produce a new array reference (prevents JustifiedGrid re-mount).
  const prevIdsRef = useRef<string>("");
  const prevFilteredRef = useRef<CachedPhoto[]>([]);

  const filteredEncrypted = useMemo(() => {
    if (!allEncryptedPhotos) return prevFilteredRef.current;
    let next = allEncryptedPhotos
      .filter((p) => !secureBlobIds.has(p.blobId))
      .filter(def.filterEncrypted);
    // "Recently Added" sorts by import order (addedAt), newest first, and caps
    // to def.limit. allEncryptedPhotos arrives takenAt-desc, so re-sort here.
    if (def.sortBy === "addedAt") {
      next = [...next].sort(
        (a, b) => (b.addedAt ?? b.takenAt ?? 0) - (a.addedAt ?? a.takenAt ?? 0),
      );
    }
    // Collapse burst stacks → one tile per burst (keep the first/newest frame in
    // the current order), so bursts STACK like the main gallery instead of
    // listing every frame. Done before the limit so "Recently Added" counts
    // bursts as one. _burstCount feeds the stack badge.
    const burstCounts = new Map<string, number>();
    for (const p of next) {
      if (p.burstId) burstCounts.set(p.burstId, (burstCounts.get(p.burstId) ?? 0) + 1);
    }
    const seenBursts = new Set<string>();
    next = next
      .filter((p) => {
        if (!p.burstId) return true;
        if (seenBursts.has(p.burstId)) return false;
        seenBursts.add(p.burstId);
        return true;
      })
      .map((p) => (p.burstId ? { ...p, _burstCount: burstCounts.get(p.burstId) } : p));
    if (def.limit !== undefined) {
      next = next.slice(0, def.limit);
    }
    // Fast equality check on blob IDs to avoid spurious re-renders
    const key = next.map((p) => p.blobId).join(",");
    if (key === prevIdsRef.current) return prevFilteredRef.current;
    prevIdsRef.current = key;
    prevFilteredRef.current = next;
    return next;
  }, [allEncryptedPhotos, secureBlobIds]);

  const photoCount = filteredEncrypted.length;

  const slideshow = usePhotoSlideshow(filteredEncrypted);

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />

      <main className="p-4">
        {/* Sub-header */}
        <div className="flex items-center gap-3 mb-4">
          <button
            onClick={() => navigate("/albums")}
            className="text-fg-muted hover:text-fg transition-colors shrink-0"
            title="Back to Albums"
          >
            <AppIcon name="back-arrow" size="w-5 h-5" />
          </button>
          <h2 className="text-xl font-semibold truncate">{def.label}</h2>
          <span className="text-fg-muted text-sm shrink-0">{photoCount} items</span>
          <SlideshowTriggers slideshow={slideshow} />
        </div>

        {loading ? (
          <GallerySkeleton />
        ) : photoCount === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">No {def.label.toLowerCase()} found</p>
          </div>
        ) : (
          <SelectablePhotoGrid
            photos={filteredEncrypted}
            viewerAlbumId={albumId}
          />
        )}
      </main>

      <SlideshowHost slideshow={slideshow} />
    </div>
  );
}
