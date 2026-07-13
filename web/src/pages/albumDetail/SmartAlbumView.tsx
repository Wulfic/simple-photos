/**
 * Smart album view — renders the synthetic albums (Recently Added, Favorites,
 * Photos, GIFs, Videos, Audio) that are computed from the local encrypted
 * photo cache rather than a user-created album manifest.
 *
 * Membership, ordering, secure-exclusion, burst-collapse and count all come
 * from the shared `useAlbumPhotos` hook, so this view is pure chrome. The
 * album definitions live in `gallery/smartAlbums`.
 */
import AppHeader from "../../components/AppHeader";
import { GallerySkeleton } from "../../components/skeletons";
import DetailHeader from "../../components/DetailHeader";
import SelectablePhotoGrid from "../../components/gallery/SelectablePhotoGrid";
import { usePhotoSlideshow } from "../../hooks/useSlideshow";
import SlideshowHost from "../../components/viewer/SlideshowHost";
import SlideshowTriggers from "../../components/viewer/SlideshowTriggers";
import { useAlbumPhotos } from "../../hooks/useAlbumPhotos";
import { SMART_ALBUM_DEFS } from "../../gallery/smartAlbums";

export default function SmartAlbumView({ albumId }: { albumId: string }) {
  const def = SMART_ALBUM_DEFS[albumId];
  const { photos, count, loading } = useAlbumPhotos(albumId);
  const slideshow = usePhotoSlideshow(photos);

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />

      <main className="p-4">
        {/* Sub-header */}
        <DetailHeader
          backTo="/albums"
          backTitle="Back to Albums"
          title={def.label}
          count={`${count} items`}
        >
          <SlideshowTriggers slideshow={slideshow} />
        </DetailHeader>

        {loading ? (
          <GallerySkeleton />
        ) : count === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">No {def.label.toLowerCase()} found</p>
          </div>
        ) : (
          <SelectablePhotoGrid photos={photos} viewerAlbumId={albumId} />
        )}
      </main>

      <SlideshowHost slideshow={slideshow} />
    </div>
  );
}
