import { request, BASE } from "./core";

// ── Trash API ────────────────────────────────────────────────────────────────

export const trashApi = {
  /** List all items in the user's trash */
  list: (params?: { after?: string; limit?: number }) => {
    const query = new URLSearchParams();
    if (params?.after) query.set("after", params.after);
    if (params?.limit) query.set("limit", params.limit.toString());
    const qs = query.toString();
    return request<{
      items: Array<{
        id: string;
        photo_id: string;
        filename: string;
        file_path: string;
        mime_type: string;
        media_type: string;
        size_bytes: number;
        width: number;
        height: number;
        duration_secs: number | null;
        taken_at: string | null;
        latitude: number | null;
        longitude: number | null;
        thumb_path: string | null;
        deleted_at: string;
        expires_at: string;
        encrypted_blob_id: string | null;
        thumbnail_blob_id: string | null;
      }>;
      next_cursor: string | null;
    }>(`/trash${qs ? `?${qs}` : ""}`);
  },

  /** Restore a photo from trash back to the gallery */
  restore: (trashId: string) =>
    request<void>(`/trash/${trashId}/restore`, { method: "POST" }),

  /** Permanently delete a single trash item */
  permanentDelete: (trashId: string) =>
    request<void>(`/trash/${trashId}`, { method: "DELETE" }),

  /** Empty the entire trash (permanently delete all items) */
  emptyTrash: () =>
    request<{ deleted: number; message: string }>("/trash", {
      method: "DELETE",
    }),

  /** Get the URL for a trashed photo's thumbnail */
  thumbUrl: (trashId: string) => `${BASE}/trash/${trashId}/thumb`,
};
