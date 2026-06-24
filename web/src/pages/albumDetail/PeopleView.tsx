/**
 * People smart album — detected face clusters. The list and per-person detail
 * views are thin configs over the shared SmartClusterList / SmartAlbumDetail
 * modules (the detail view enables rename).
 */
import { api } from "../../api/client";
import SmartClusterList from "./SmartClusterList";
import SmartAlbumDetail from "./SmartAlbumDetail";
import { resolvePhotosByServerId } from "./resolveServerPhotos";

const PersonIcon = (
  <svg className="w-10 h-10 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z" />
  </svg>
);

export function PeopleView() {
  return (
    <SmartClusterList
      title="People"
      emptyTitle="No faces detected yet"
      emptyHint="Enable AI processing in Settings to detect faces"
      variant="avatar"
      placeholder={PersonIcon}
      load={() => api.ai.listFaceClusters()}
      toCard={(cluster) => ({
        key: cluster.id,
        photoId: cluster.representative,
        href: `/albums/smart-people/${cluster.id}`,
        title: cluster.label || "Unknown Person",
        alt: cluster.label || "Unknown",
        meta: (
          <p className="text-xs text-fg-muted text-center">
            {cluster.photo_count} photo{cluster.photo_count !== 1 ? "s" : ""}
          </p>
        ),
      })}
    />
  );
}

export function PersonDetailView({ clusterId }: { clusterId: number }) {
  return (
    <SmartAlbumDetail
      reloadKey={clusterId}
      defaultTitle="Person"
      backTo="/albums/smart-people"
      backLabel="People"
      viewerAlbumId={`smart-people/${clusterId}`}
      emptyMessage="No photos found for this person"
      load={async ({ setTitle, setRenameValue }) => {
        const clusters = await api.ai.listFaceClusters();
        const cluster = clusters.find((c) => c.id === clusterId);
        if (cluster) {
          setTitle(cluster.label || "Unknown Person");
          setRenameValue(cluster.label || "");
        }
        const detections = await api.ai.listClusterPhotos(clusterId);
        const photoIds = [...new Set(detections.map((d) => d.photo_id))];
        return resolvePhotosByServerId(photoIds);
      }}
      onRename={async (value) => {
        await api.ai.renameFaceCluster(clusterId, value);
        return value || "Unknown Person";
      }}
    />
  );
}
