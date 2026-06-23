/**
 * Album detail page — renders photos for a user-created album or a
 * "smart album" (Favorites, Photos, GIFs, Videos, Audio, People, Pets,
 * Memories, Trips).
 *
 * This file is just the router: it inspects the route params and delegates
 * to the matching view in ./albumDetail/. The view implementations (album
 * CRUD, photo addition/removal, cover photo selection, sharing, face/pet
 * clusters, geo memories/trips) live in their own files there.
 */
import { useParams } from "react-router-dom";
import SmartAlbumView, { isSmartAlbum } from "./albumDetail/SmartAlbumView";
import RegularAlbumView from "./albumDetail/RegularAlbumView";
import { PeopleView, PersonDetailView } from "./albumDetail/PeopleView";
import { PetsView, PetDetailView } from "./albumDetail/PetsView";
import { MemoriesView, MemoryDetailView } from "./albumDetail/MemoriesView";
import { TripsView, TripDetailView } from "./albumDetail/TripsView";

export default function AlbumDetail() {
  const { albumId, subId } = useParams<{ albumId: string; subId?: string }>();

  // ── Special smart album views ───────────────────────────────────────────
  if (albumId === "smart-people") {
    if (subId) return <PersonDetailView clusterId={Number(subId)} />;
    return <PeopleView />;
  }
  if (albumId === "smart-pets") {
    if (subId) return <PetDetailView clusterId={Number(subId)} />;
    return <PetsView />;
  }
  if (albumId === "smart-memories") {
    if (subId) return <MemoryDetailView memoryId={subId} />;
    return <MemoriesView />;
  }
  if (albumId === "smart-trips") {
    if (subId) return <TripDetailView tripId={subId} />;
    return <TripsView />;
  }

  // ── Smart album rendering (delegates to a separate sub-component) ───────
  if (isSmartAlbum(albumId)) {
    return <SmartAlbumView albumId={albumId} />;
  }

  return <RegularAlbumView albumId={albumId} />;
}
