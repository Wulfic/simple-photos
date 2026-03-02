import { request } from "./core";

// ── Shared Albums API ────────────────────────────────────────────────────────

export const sharingApi = {
  /** List shared albums the user owns or is a member of */
  listAlbums: () =>
    request<
      Array<{
        id: string;
        name: string;
        owner_username: string;
        is_owner: boolean;
        photo_count: number;
        member_count: number;
        created_at: string;
      }>
    >("/sharing/albums"),

  /** Create a new shared album */
  createAlbum: (name: string) =>
    request<{ id: string; name: string; created_at: string }>(
      "/sharing/albums",
      {
        method: "POST",
        body: JSON.stringify({ name }),
      }
    ),

  /** Delete a shared album (owner only) */
  deleteAlbum: (albumId: string) =>
    request<void>(`/sharing/albums/${albumId}`, { method: "DELETE" }),

  /** List members of a shared album */
  listMembers: (albumId: string) =>
    request<
      Array<{
        id: string;
        user_id: string;
        username: string;
        added_at: string;
      }>
    >(`/sharing/albums/${albumId}/members`),

  /** Add a member to a shared album */
  addMember: (albumId: string, userId: string) =>
    request<{ member_id: string; user_id: string }>(
      `/sharing/albums/${albumId}/members`,
      {
        method: "POST",
        body: JSON.stringify({ user_id: userId }),
      }
    ),

  /** Remove a member from a shared album */
  removeMember: (albumId: string, userId: string) =>
    request<void>(`/sharing/albums/${albumId}/members/${userId}`, {
      method: "DELETE",
    }),

  /** List photos in a shared album */
  listPhotos: (albumId: string) =>
    request<
      Array<{
        id: string;
        photo_ref: string;
        ref_type: string;
        added_at: string;
      }>
    >(`/sharing/albums/${albumId}/photos`),

  /** Add a photo to a shared album */
  addPhoto: (albumId: string, photoRef: string, refType: "plain" | "blob" = "plain") =>
    request<{ photo_id: string }>(
      `/sharing/albums/${albumId}/photos`,
      {
        method: "POST",
        body: JSON.stringify({ photo_ref: photoRef, ref_type: refType }),
      }
    ),

  /** Remove a photo from a shared album */
  removePhoto: (albumId: string, photoId: string) =>
    request<void>(`/sharing/albums/${albumId}/photos/${photoId}`, {
      method: "DELETE",
    }),

  /** List all users for the member picker */
  listUsers: () =>
    request<Array<{ id: string; username: string }>>("/sharing/users"),
};
