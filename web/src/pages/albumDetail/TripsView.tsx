/**
 * Trips smart album — auto-generated multi-day location albums. The list and
 * per-trip detail views are thin configs over the shared SmartClusterList /
 * SmartAlbumDetail modules.
 */
import { api } from "../../api/client";
import SmartClusterList from "./SmartClusterList";
import SmartAlbumDetail from "./SmartAlbumDetail";
import { resolveServerPhotos } from "./resolveServerPhotos";

const TripIcon = (
  <svg className="w-8 h-8 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M9 6.75V15m6-6v8.25m.503 3.498 4.875-2.437c.381-.19.622-.58.622-1.006V4.82c0-.836-.88-1.38-1.628-1.006l-3.869 1.934c-.317.159-.69.159-1.006 0L9.503 3.252a1.125 1.125 0 0 0-1.006 0L3.622 5.689C3.24 5.88 3 6.27 3 6.695V19.18c0 .836.88 1.38 1.628 1.006l3.869-1.934c.317-.159.69-.159 1.006 0l4.994 2.497c.317.158.69.158 1.006 0Z" />
  </svg>
);

export function TripsView() {
  return (
    <SmartClusterList
      title="Trips"
      emptyTitle="No trips yet"
      emptyHint="Trips are auto-generated when you have photos from the same location across multiple days"
      variant="card"
      placeholder={TripIcon}
      load={() => api.geo.listTrips()}
      toCard={(trip) => ({
        key: trip.id,
        photoId: trip.first_photo_id,
        href: `/albums/smart-trips/${trip.id}`,
        title: trip.city,
        alt: trip.name,
        meta: (
          <>
            <p className="text-xs text-fg-muted truncate">{trip.date_label}</p>
            <p className="text-xs text-fg-muted">
              {trip.photo_count} photo{trip.photo_count !== 1 ? "s" : ""} · {trip.day_count} day{trip.day_count !== 1 ? "s" : ""} · {trip.country}
            </p>
          </>
        ),
      })}
    />
  );
}

export function TripDetailView({ tripId }: { tripId: string }) {
  return (
    <SmartAlbumDetail
      reloadKey={tripId}
      defaultTitle="Trip"
      backTo="/albums/smart-trips"
      backLabel="Trips"
      viewerAlbumId={`smart-trips/${tripId}`}
      emptyMessage="No photos found for this trip"
      load={async ({ setTitle }) => {
        const trips = await api.geo.listTrips();
        const trip = trips.find((t) => t.id === tripId);
        if (trip) setTitle(`${trip.city} · ${trip.date_label}`);
        const summaries = await api.geo.listTripPhotos(tripId);
        return resolveServerPhotos(summaries);
      }}
    />
  );
}
