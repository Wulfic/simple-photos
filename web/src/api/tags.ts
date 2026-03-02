import { request } from "./core";

// ── Tags API ─────────────────────────────────────────────────────────────────

export const tagsApi = {
  /** List all unique tags for the current user */
  list: () =>
    request<{ tags: string[] }>("/tags"),

  /** Get tags on a specific photo */
  getPhotoTags: (photoId: string) =>
    request<{ photo_id: string; tags: string[] }>(`/photos/${photoId}/tags`),

  /** Add a tag to a photo */
  add: (photoId: string, tag: string) =>
    request<void>(`/photos/${photoId}/tags`, {
      method: "POST",
      body: JSON.stringify({ tag }),
    }),

  /** Remove a tag from a photo */
  remove: (photoId: string, tag: string) =>
    request<void>(`/photos/${photoId}/tags`, {
      method: "DELETE",
      body: JSON.stringify({ tag }),
    }),
};

// ── Search API ───────────────────────────────────────────────────────────────

export const searchApi = {
  /** Search photos by tag, filename, date, location, or media type */
  query: (q: string, limit?: number) => {
    const params = new URLSearchParams({ q });
    if (limit) params.set("limit", limit.toString());
    return request<{
      results: Array<{
        id: string;
        filename: string;
        media_type: string;
        mime_type: string;
        thumb_path: string | null;
        created_at: string;
        taken_at: string | null;
        latitude: number | null;
        longitude: number | null;
        width: number | null;
        height: number | null;
        tags: string[];
      }>;
    }>(`/search?${params.toString()}`);
  },
};
