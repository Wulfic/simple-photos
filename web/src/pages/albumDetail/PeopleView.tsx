/**
 * People smart album — detected face clusters. The list and per-person detail
 * views are thin configs over the shared SmartClusterList / SmartAlbumDetail
 * modules (the detail view enables rename).
 */
import { useSearchParams } from "react-router-dom";
import { api } from "../../api/client";
import { aiApi, type FaceCluster } from "../../api/ai";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import SmartClusterList from "./SmartClusterList";
import SmartAlbumDetail from "./SmartAlbumDetail";
import { resolvePhotosByServerId } from "./resolveServerPhotos";
import { computeFaceCropStyle } from "../../utils/thumbnailCss";

/** Build the face-zoom style for a cluster tile, when the server sent a bbox. */
function faceCropStyle(cluster: FaceCluster) {
  if (cluster.rep_bbox_w == null || cluster.rep_bbox_h == null) return undefined;
  return computeFaceCropStyle({
    x: cluster.rep_bbox_x ?? 0,
    y: cluster.rep_bbox_y ?? 0,
    w: cluster.rep_bbox_w,
    h: cluster.rep_bbox_h,
  });
}

const PersonIcon = (
  <svg className="w-10 h-10 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z" />
  </svg>
);

export function PeopleView() {
  const [searchParams] = useSearchParams();
  const navigate = useAppNavigate();

  // Assign mode: launched from the photo info panel's "Select Person" button.
  // `detection` is the face to move; picking a person reassigns it and returns.
  const assigning = searchParams.get("assign") === "1";
  const detectionId = Number(searchParams.get("detection"));
  const assignDetection = assigning && Number.isFinite(detectionId) ? detectionId : null;

  const handlePick = async (cluster: FaceCluster) => {
    if (assignDetection == null) return;
    try {
      await aiApi.assignFace(assignDetection, cluster.id);
    } catch (e) {
      console.error("Failed to reassign face", e);
    } finally {
      // Return to the photo the user came from.
      navigate(-1);
    }
  };

  return (
    <SmartClusterList
      title={assignDetection != null ? "Assign to person" : "People"}
      emptyTitle="No faces detected yet"
      emptyHint="Enable AI processing in Settings to detect faces"
      variant="avatar"
      placeholder={PersonIcon}
      load={() => api.ai.listFaceClusters()}
      onCardClick={assignDetection != null ? handlePick : undefined}
      notice={
        assignDetection != null ? (
          <div className="mb-4 rounded-lg border border-accent-500/30 bg-accent-500/10 px-4 py-3 text-sm text-fg">
            Choose the correct person for this face.
          </div>
        ) : undefined
      }
      toCard={(cluster) => ({
        key: cluster.id,
        photoId: cluster.representative,
        href: `/albums/smart-people/${cluster.id}`,
        title: cluster.label || "Unknown Person",
        alt: cluster.label || "Unknown",
        imgStyle: faceCropStyle(cluster),
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
