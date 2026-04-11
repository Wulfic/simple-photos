/**
 * Blob storage API client — upload, list, download, and delete encrypted blobs.
 *
 * Used in encrypted mode where the client encrypts media before upload and the
 * server stores opaque ciphertext. Soft-delete moves blobs to trash with
 * 30-day retention.
 *
 * Maps to server routes: `/api/blobs/*`.
 */
import { request, downloadRaw, BASE } from "./core";

// ── Blob Storage API ─────────────────────────────────────────────────────────

export const blobsApi = {
  upload: (data: ArrayBuffer, blobType: string, clientHash?: string, contentHash?: string) => {
    const headers: Record<string, string> = {
      "X-Blob-Type": blobType,
      "X-Blob-Size": data.byteLength.toString(),
    };
    if (clientHash) headers["X-Client-Hash"] = clientHash;
    if (contentHash) headers["X-Content-Hash"] = contentHash;

    return request<{
      blob_id: string;
      upload_time: string;
      size: number;
    }>("/blobs", {
      method: "POST",
      headers,
      body: data,
    });
  },

  list: (params?: {
    blob_type?: string;
    after?: string;
    limit?: number;
  }) => {
    const query = new URLSearchParams();
    if (params?.blob_type) query.set("blob_type", params.blob_type);
    if (params?.after) query.set("after", params.after);
    if (params?.limit) query.set("limit", params.limit.toString());
    const qs = query.toString();
    return request<{
      blobs: Array<{
        id: string;
        blob_type: string;
        size_bytes: number;
        client_hash: string | null;
        upload_time: string;
        content_hash: string | null;
      }>;
      next_cursor: string | null;
    }>(`/blobs${qs ? `?${qs}` : ""}`);
  },

  download: async (blobId: string, signal?: AbortSignal): Promise<ArrayBuffer> => {
    return downloadRaw(`${BASE}/blobs/${blobId}`, signal);
  },

  delete: (blobId: string) =>
    request<void>(`/blobs/${blobId}`, { method: "DELETE" }),

  /** Soft-delete a blob to trash (encrypted mode) */
  softDelete: (blobId: string, meta: {
    thumbnail_blob_id?: string;
    filename: string;
    mime_type: string;
    media_type?: string;
    size_bytes?: number;
    width?: number;
    height?: number;
    duration_secs?: number;
    taken_at?: string;
  }) =>
    request<{ trash_id: string; expires_at: string }>(
      `/blobs/${blobId}/trash`,
      { method: "POST", body: JSON.stringify(meta) }
    ),
};
