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
}

export interface FaceCluster {
  id: number;
  label: string | null;
  photo_count: number;
  representative: string | null;
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

  /** List unique object classes detected */
  listObjectClasses: () => request<ObjectClassSummary[]>("/ai/objects"),

  /** List photos containing a specific object type */
  listObjectPhotos: (className: string) =>
    request<ObjectDetectionRecord[]>(
      `/ai/objects/${encodeURIComponent(className)}/photos`
    ),
};
