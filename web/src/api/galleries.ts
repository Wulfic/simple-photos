/**
 * Secure galleries API client — password-protected photo galleries.
 *
 * Create, unlock, list, and manage items in secure galleries. Unlocking
 * returns a time-limited token that must be sent as `X-Gallery-Token`
 * to access gallery items.
 *
 * Maps to server routes: `/api/galleries/secure/*`.
 */
import { request } from "./core";

// ── Secure Galleries API ─────────────────────────────────────────────────────

export const secureGalleriesApi = {
  list: () =>
    request<{
      galleries: Array<{
        id: string;
        name: string;
        created_at: string;
        item_count: number;
      }>;
    }>("/galleries/secure"),

  create: (name: string) =>
    request<{ gallery_id: string; name: string }>("/galleries/secure", {
      method: "POST",
      body: JSON.stringify({ name }),
    }),

  delete: (galleryId: string) =>
    request<void>(`/galleries/secure/${galleryId}`, {
      method: "DELETE",
    }),

  /** Unlock all secure galleries using the user's account password */
  unlock: (password: string) =>
    request<{ gallery_token: string; expires_in: number }>(
      `/galleries/secure/unlock`,
      {
        method: "POST",
        body: JSON.stringify({ password }),
      }
    ),

  listItems: (galleryId: string, galleryToken: string) =>
    request<{
      items: Array<{ id: string; blob_id: string; added_at: string; encrypted_thumb_blob_id?: string | null }>;
    }>(`/galleries/secure/${galleryId}/items`, {
      headers: { "X-Gallery-Token": galleryToken },
    }),

  addItem: (galleryId: string, blobId: string) =>
    request<{ item_id: string; new_blob_id: string }>(
      `/galleries/secure/${galleryId}/items`,
      {
        method: "POST",
        body: JSON.stringify({ blob_id: blobId }),
      }
    ),

  /** Get all blob IDs across all secure galleries (for filtering from main gallery) */
  secureBlobIds: () =>
    request<{ blob_ids: string[] }>("/galleries/secure/blob-ids"),
};
