/**
 * Trips smart album — auto-generated multi-day location albums. Lists every
 * trip and renders the per-trip photo grid (server-resolved photos).
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

// ── Trips View (list of all multi-day smart location albums) ─────────────────

export function TripsView() {
  const navigate = useAppNavigate();
  const [trips, setTrips] = useState<Array<{
    id: string; name: string; city: string; country: string;
    country_code: string; start_date: string; end_date: string;
    date_label: string; photo_count: number; day_count: number;
    first_photo_id: string | null; first_thumb_path: string | null;
  }>>([]);
  const [loading, setLoading] = useState(true);
  const thumbUrls = useIdbThumbnailMap(
    trips.map((t) => ({ key: t.id, photoId: t.first_photo_id })),
  );

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = await api.geo.listTrips();
        if (!cancelled) setTrips(data);
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
          <h2 className="text-xl font-semibold">Trips</h2>
        </div>

        {loading ? (
          <GallerySkeleton />
        ) : trips.length === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">No trips yet</p>
            <p className="text-fg-muted text-sm mt-1">
              Trips are auto-generated when you have photos from the same location across multiple days
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-4">
            {trips.map((trip) => (
              <div
                key={trip.id}
                onClick={() => navigate(`/albums/smart-trips/${trip.id}`)}
                className="card card-interactive cursor-pointer overflow-hidden"
              >
                <div className="aspect-video bg-surface-raised flex items-center justify-center overflow-hidden">
                  {thumbUrls[trip.id] ? (
                    <img
                      src={thumbUrls[trip.id]}
                      alt={trip.name}
                      className="w-full h-full object-cover"
                    />
                  ) : (
                    <svg className="w-8 h-8 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M9 6.75V15m6-6v8.25m.503 3.498 4.875-2.437c.381-.19.622-.58.622-1.006V4.82c0-.836-.88-1.38-1.628-1.006l-3.869 1.934c-.317.159-.69.159-1.006 0L9.503 3.252a1.125 1.125 0 0 0-1.006 0L3.622 5.689C3.24 5.88 3 6.27 3 6.695V19.18c0 .836.88 1.38 1.628 1.006l3.869-1.934c.317-.159.69-.159 1.006 0l4.994 2.497c.317.158.69.158 1.006 0Z" />
                    </svg>
                  )}
                </div>
                <div className="p-3">
                  <p className="font-medium text-sm truncate">{trip.city}</p>
                  <p className="text-xs text-fg-muted truncate">{trip.date_label}</p>
                  <p className="text-xs text-fg-muted">
                    {trip.photo_count} photo{trip.photo_count !== 1 ? "s" : ""} · {trip.day_count} day{trip.day_count !== 1 ? "s" : ""} · {trip.country}
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

// ── Trip Detail View ──────────────────────────────────────────────────────────

export function TripDetailView({ tripId }: { tripId: string }) {
  const navigate = useAppNavigate();
  const [tripName, setTripName] = useState("Trip");
  const [photos, setPhotos] = useState<CachedPhoto[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const trips = await api.geo.listTrips();
        const trip = trips.find(t => t.id === tripId);
        if (!cancelled && trip) setTripName(`${trip.city} · ${trip.date_label}`);

        const summaries = await api.geo.listTripPhotos(tripId);
        const found = await resolveServerPhotos(summaries);
        if (!cancelled) setPhotos(found);
      } catch { /* trip may not exist */ }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, [tripId]);

  const slideshow = usePhotoSlideshow(photos);

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />
      <main className="p-4">
        <div className="flex items-center gap-3 mb-4">
          <button
            onClick={() => navigate("/albums/smart-trips")}
            className="text-fg-muted hover:text-fg transition-colors shrink-0"
            title="Back to Trips"
          >
            <AppIcon name="back-arrow" size="w-5 h-5" />
          </button>
          <h2 className="text-xl font-semibold truncate">{tripName}</h2>
          <span className="text-fg-muted text-sm shrink-0">{photos.length} photos</span>
          <SlideshowTriggers slideshow={slideshow} />
        </div>

        {loading ? (
          <GallerySkeleton />
        ) : photos.length === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">No photos found for this trip</p>
          </div>
        ) : (
          <SelectablePhotoGrid
            photos={photos}
            viewerAlbumId={`smart-trips/${tripId}`}
            onDeleted={(ids) => setPhotos((prev) => prev.filter((p) => !ids.includes(p.blobId)))}
          />
        )}
      </main>

      <SlideshowHost slideshow={slideshow} />
    </div>
  );
}
