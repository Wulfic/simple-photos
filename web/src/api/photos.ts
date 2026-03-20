/**
 * Photos API client — list, upload, download, favorite, crop, duplicate,
 * edit copies, and encrypted sync operations.
 *
 * Blob IDs reference encrypted data. URL builders produce authenticated
 * URLs for `<img>` / `<video>` elements that can't set headers.
 *
 * Maps to server routes: `/api/photos/*`.
 */
import { request, downloadRaw, BASE } from "./core";

// ── Photos API ───────────────────────────────────────────────────────────────

export const photosApi = {
  list: (params?: { after?: string; limit?: number; media_type?: string; favorites_only?: boolean }) => {
    const query = new URLSearchParams();
    if (params?.after) query.set("after", params.after);
    if (params?.limit) query.set("limit", params.limit.toString());
    if (params?.media_type) query.set("media_type", params.media_type);
    if (params?.favorites_only) query.set("favorites_only", "true");
    const qs = query.toString();
    return request<{
      photos: Array<{
        id: string;
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
        created_at: string;
        is_favorite: boolean;
        crop_metadata: string | null;
        camera_model: string | null;
        photo_hash: string | null;
      }>;
      next_cursor: string | null;
    }>(`/photos${qs ? `?${qs}` : ""}`);
  },

  register: (data: {
    filename: string;
    file_path: string;
    mime_type: string;
    size_bytes: number;
    media_type?: string;
    width?: number;
    height?: number;
    duration_secs?: number;
    taken_at?: string;
    latitude?: number;
    longitude?: number;
  }) =>
    request<{ photo_id: string; thumb_path: string }>("/photos/register", {
      method: "POST",
      body: JSON.stringify(data),
    }),

  /** Get the URL for serving a plain photo file (original format) */
  fileUrl: (photoId: string) => `${BASE}/photos/${photoId}/file`,

  /** Get the URL for serving a plain photo thumbnail */
  thumbUrl: (photoId: string) => `${BASE}/photos/${photoId}/thumb`,

  /** Get the URL for serving the media in a browser-compatible format.
   *  Only browser-native formats are stored, so this serves the original file. */
  webUrl: (photoId: string) => `${BASE}/photos/${photoId}/web`,

  /** Build a web-media URL with an inline auth token.
   *  Use this for `<video>`/`<audio>` `src` attributes so the browser can
   *  issue native HTTP Range requests (seeking, streaming).  The token is
   *  passed as a query parameter because media elements cannot set custom
   *  headers. */
  streamUrl: (photoId: string, token: string) =>
    `${BASE}/photos/${photoId}/web?token=${encodeURIComponent(token)}`,

  /** Download a plain photo file as raw binary, with 401 refresh retry */
  downloadFile: (photoId: string) =>
    downloadRaw(`${BASE}/photos/${photoId}/file`),

  /** Download a plain photo thumbnail as raw binary, with 401 refresh retry */
  downloadThumb: (photoId: string) =>
    downloadRaw(`${BASE}/photos/${photoId}/thumb`),

  delete: (photoId: string) =>
    request<void>(`/photos/${photoId}`, { method: "DELETE" }),

  /** Toggle the is_favorite flag on a photo */
  toggleFavorite: (photoId: string) =>
    request<{ id: string; is_favorite: boolean }>(`/photos/${photoId}/favorite`, {
      method: "PUT",
    }),

  /** Set or clear crop metadata for a photo */
  setCrop: (photoId: string, cropMetadata: string | null) =>
    request<{ id: string; crop_metadata: string | null }>(`/photos/${photoId}/crop`, {
      method: "PUT",
      body: JSON.stringify({ crop_metadata: cropMetadata }),
    }),

  /** Duplicate a photo (Save as Copy) — creates a new photos row sharing
   *  the same underlying file but with its own crop/edit metadata. */
  duplicate: (photoId: string, cropMetadata: string | null) =>
    request<{ id: string; source_photo_id: string; filename: string; crop_metadata: object | null }>(
      `/photos/${photoId}/duplicate`,
      {
        method: "POST",
        body: JSON.stringify({ crop_metadata: cropMetadata }),
      },
    ),

  /** Create a metadata-only "copy" of a photo/video/audio */
  createEditCopy: (photoId: string, editMetadata: string, name?: string) =>
    request<{ id: string; photo_id: string; name: string; edit_metadata: object }>(
      `/photos/${photoId}/copies`,
      {
        method: "POST",
        body: JSON.stringify({ edit_metadata: editMetadata, name }),
      },
    ),

  /** List all edit copies for a photo */
  listEditCopies: (photoId: string) =>
    request<{
      copies: Array<{
        id: string;
        name: string;
        edit_metadata: object;
        created_at: string;
      }>;
    }>(`/photos/${photoId}/copies`),

  /** Delete a single edit copy */
  deleteEditCopy: (photoId: string, copyId: string) =>
    request<{ ok: boolean }>(`/photos/${photoId}/copies/${copyId}`, {
      method: "DELETE",
    }),

  /** Lightweight encrypted-mode sync — returns photo metadata from the photos table
   *  without requiring blob decryption. Both web and mobile use this for consistent sort order. */
  encryptedSync: (params?: { after?: string; limit?: number }) => {
    const query = new URLSearchParams();
    if (params?.after) query.set("after", params.after);
    if (params?.limit) query.set("limit", params.limit.toString());
    const qs = query.toString();
    return request<{
      photos: Array<{
        id: string;
        filename: string;
        mime_type: string;
        media_type: string;
        size_bytes: number;
        width: number;
        height: number;
        duration_secs: number | null;
        taken_at: string | null;
        created_at: string;
        encrypted_blob_id: string | null;
        encrypted_thumb_blob_id: string | null;
        is_favorite: boolean;
        crop_metadata: string | null;
        photo_hash: string | null;
      }>;
      next_cursor: string | null;
    }>(`/photos/encrypted-sync${qs ? `?${qs}` : ""}`);
  },


};
