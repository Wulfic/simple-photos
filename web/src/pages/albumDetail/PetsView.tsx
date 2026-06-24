/**
 * Pets smart album — detected animal clusters. The list and per-pet detail
 * views are thin configs over the shared SmartClusterList / SmartAlbumDetail
 * modules (the detail view enables rename).
 */
import { useRef } from "react";
import { api } from "../../api/client";
import { useAuthStore } from "../../store/auth";
import SmartClusterList from "./SmartClusterList";
import SmartAlbumDetail from "./SmartAlbumDetail";
import { resolvePhotosByServerId } from "./resolveServerPhotos";

const PetIcon = (
  <svg className="w-10 h-10 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M12 6.75a.75.75 0 110-1.5.75.75 0 010 1.5zM12 12.75a.75.75 0 110-1.5.75.75 0 010 1.5zM12 18.75a.75.75 0 110-1.5.75.75 0 010 1.5z" />
  </svg>
);

export function PetsView() {
  const { accessToken } = useAuthStore();
  return (
    <SmartClusterList
      title="Pets"
      emptyTitle="No pets detected yet"
      emptyHint="Enable AI processing in Settings to detect pets in your photos"
      variant="avatar"
      titleClassName="capitalize"
      placeholder={PetIcon}
      load={() => api.ai.listPetClusters()}
      fallbackUrl={(id) =>
        accessToken ? `${api.photos.thumbUrl(id)}?token=${accessToken}` : api.photos.thumbUrl(id)}
      toCard={(cluster) => ({
        key: cluster.id,
        photoId: cluster.representative,
        href: `/albums/smart-pets/${cluster.id}`,
        title: cluster.label || `Unknown ${cluster.species}`,
        alt: cluster.label || cluster.species,
        meta: (
          <p className="text-xs text-fg-muted text-center">
            {cluster.photo_count} photo{cluster.photo_count !== 1 ? "s" : ""}
          </p>
        ),
      })}
    />
  );
}

export function PetDetailView({ clusterId }: { clusterId: number }) {
  const speciesRef = useRef("");
  return (
    <SmartAlbumDetail
      reloadKey={clusterId}
      defaultTitle="Pet"
      titleClassName="capitalize"
      backTo="/albums/smart-pets"
      backLabel="Pets"
      viewerAlbumId={`smart-pets/${clusterId}`}
      emptyMessage="No photos found for this pet"
      load={async ({ setTitle, setRenameValue }) => {
        const clusters = await api.ai.listPetClusters();
        const cluster = clusters.find((c) => c.id === clusterId);
        if (cluster) {
          speciesRef.current = cluster.species;
          setTitle(cluster.label || `Unknown ${cluster.species}`);
          setRenameValue(cluster.label || "");
        }
        const detections = await api.ai.listPetClusterPhotos(clusterId);
        const photoIds = [...new Set(detections.map((d) => d.photo_id))];
        return resolvePhotosByServerId(photoIds);
      }}
      onRename={async (value) => {
        await api.ai.renamePetCluster(clusterId, value);
        return value || `Unknown ${speciesRef.current}`;
      }}
    />
  );
}
