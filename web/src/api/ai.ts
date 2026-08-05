/**
 * AI recognition API client.
 *
 * Covers status queries, face cluster management (list, rename, merge, split),
 * object detection results, and processing control (enable/disable, reprocess).
 */

import { request } from "./core";

export interface AiStatus {
  enabled: boolean;
  gpu_available: boolean;
  photos_processed: number;
  photos_pending: number;
  face_detections: number;
  face_clusters: number;
  object_detections: number;
  pet_detections: number;
  pet_clusters: number;
}

export interface FaceCluster {
  id: number;
  label: string | null;
  photo_count: number;
  representative: string | null;
  /** Representative face bbox (normalised 0–1) for cropping the People tile.
   *  Null when the server couldn't resolve a detection for the representative. */
  rep_bbox_x: number | null;
  rep_bbox_y: number | null;
  rep_bbox_w: number | null;
  rep_bbox_h: number | null;
  created_at: string;
  updated_at: string;
}

export interface FaceDetectionRecord {
  id: number;
  photo_id: string;
  cluster_id: number | null;
  bbox_x: number;
  bbox_y: number;
  bbox_w: number;
  bbox_h: number;
  confidence: number;
  created_at: string;
}

/** One face detected in a specific photo, with its current person label. */
export interface PhotoFace {
  id: number;
  cluster_id: number | null;
  cluster_label: string | null;
  bbox_x: number;
  bbox_y: number;
  bbox_w: number;
  bbox_h: number;
  confidence: number;
}

export interface ObjectClassSummary {
  class_name: string;
  photo_count: number;
  avg_confidence: number;
}

export interface ObjectDetectionRecord {
  id: number;
  photo_id: string;
  class_name: string;
  confidence: number;
  bbox_x: number;
  bbox_y: number;
  bbox_w: number;
  bbox_h: number;
  created_at: string;
}

export interface PetCluster {
  id: number;
  label: string | null;
  species: string;
  photo_count: number;
  representative: string | null;
  /** Representative animal bbox (normalised 0–1) for framing the Pets tile.
   *  Same contract as FaceCluster's — null when the photo predates the box
   *  being stored (migration 039) or no detection resolved. */
  rep_bbox_x: number | null;
  rep_bbox_y: number | null;
  rep_bbox_w: number | null;
  rep_bbox_h: number | null;
  created_at: string;
  updated_at: string;
}

export interface PetDetectionRecord {
  id: number;
  photo_id: string;
  cluster_id: number | null;
  species: string;
  confidence: number;
  created_at: string;
}

export const aiApi = {
  /** Get AI processing status and capabilities */
  getStatus: () => request<AiStatus>("/ai/status"),

  /** Enable or disable AI processing */
  toggle: (enabled: boolean) =>
    request<void>("/ai/toggle", {
      method: "POST",
      body: JSON.stringify({ enabled }),
    }),

  /** Trigger reprocessing of all or specific photos */
  reprocess: (photoIds?: string[]) =>
    request<{ cleared: number; message: string }>("/ai/reprocess", {
      method: "POST",
      body: JSON.stringify({ photo_ids: photoIds }),
    }),

  /** List all face clusters */
  listFaceClusters: () => request<FaceCluster[]>("/ai/faces"),

  /** List photos in a specific face cluster */
  listClusterPhotos: (clusterId: number) =>
    request<FaceDetectionRecord[]>(`/ai/faces/${clusterId}/photos`),

  /** Rename a face cluster */
  renameFaceCluster: (clusterId: number, name: string) =>
    request<void>(`/ai/faces/${clusterId}/name`, {
      method: "PUT",
      body: JSON.stringify({ name }),
    }),

  /** Merge multiple face clusters into one */
  mergeFaceClusters: (clusterIds: number[]) =>
    request<{ merged_into: number; photo_count: number }>("/ai/faces/merge", {
      method: "POST",
      body: JSON.stringify({ cluster_ids: clusterIds }),
    }),

  /** Split face detections into a new cluster */
  splitFaceCluster: (detectionIds: number[]) =>
    request<{ new_cluster_id: number; detection_count: number }>(
      "/ai/faces/split",
      {
        method: "POST",
        body: JSON.stringify({ detection_ids: detectionIds }),
      }
    ),

  /** List the faces detected in one photo (with current person labels) */
  listPhotoFaces: (photoId: string) =>
    request<PhotoFace[]>(`/ai/photos/${encodeURIComponent(photoId)}/faces`),

  /** Manually reassign a face detection to a person (cluster) */
  assignFace: (detectionId: number, clusterId: number) =>
    request<{ detection_id: number; cluster_id: number; photo_id: string }>(
      "/ai/faces/assign",
      {
        method: "POST",
        body: JSON.stringify({ detection_id: detectionId, cluster_id: clusterId }),
      }
    ),

  /** List unique object classes detected */
  listObjectClasses: () => request<ObjectClassSummary[]>("/ai/objects"),

  /** List photos containing a specific object type */
  listObjectPhotos: (className: string) =>
    request<ObjectDetectionRecord[]>(
      `/ai/objects/${encodeURIComponent(className)}/photos`
    ),

  /** List all pet clusters */
  listPetClusters: () => request<PetCluster[]>("/ai/pets"),

  /** List photos in a specific pet cluster */
  listPetClusterPhotos: (clusterId: number) =>
    request<PetDetectionRecord[]>(`/ai/pets/${clusterId}/photos`),

  /** Rename a pet cluster */
  renamePetCluster: (clusterId: number, name: string) =>
    request<void>(`/ai/pets/${clusterId}/name`, {
      method: "PUT",
      body: JSON.stringify({ name }),
    }),

  /** Merge multiple pet clusters into one */
  mergePetClusters: (clusterIds: number[]) =>
    request<{ merged_into: number; photo_count: number }>("/ai/pets/merge", {
      method: "POST",
      body: JSON.stringify({ cluster_ids: clusterIds }),
    }),
};
