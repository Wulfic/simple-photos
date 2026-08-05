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
import type { Rendition } from "../gallery/renditionChoice";

/**
 * A single item in a secure gallery. `gallery_id` (the owning album) is present
 * on both the per-gallery and aggregate responses; `gallery_name` is only set
 * by the aggregate `/galleries/secure/items` endpoint (used for the smart-album
 * detail header).
 */
export type SecureGalleryItem = {
  id: string;
  blob_id: string;
  added_at: string;
  gallery_id: string;
  gallery_name?: string | null;
  /**
   * Every secure album that holds this photo (Z1), oldest membership first.
   * Published by **both** the per-album and aggregate feeds, with the same
   * meaning on each, so a client never has to know which endpoint it came from.
   *
   * **Absent or empty means UNKNOWN, not "no other album."** The server's own
   * comment says so: a miss is unreachable by construction, so an empty array
   * can only be an older server that does not publish memberships. Reading it as
   * 0 is what made the removal dialog promise a photo would return to the
   * regular gallery when it would stay secured. Resolve it through
   * `otherSecureAlbumCount`, which encodes that distinction.
   */
  galleries?: Array<{ id: string; name: string }> | null;
  encrypted_thumb_blob_id?: string | null;
  width?: number | null;
  height?: number | null;
  media_type?: string | null;
  photo_subtype?: string | null;
  burst_id?: string | null;
  duration_secs?: number | null;
  motion_video_blob_id?: string | null;
  /**
   * Non-destructive edit metadata (crop / brightness / rotate / trim), same
   * JSON shape as a regular photo's `cropData`. Stored on the secure item row
   * itself (#31) so it never leaks onto the hidden original. `null`/absent = no
   * edits. Applied at display time by the viewer + tiles, exactly like a
   * regular photo — no re-render of the encrypted blob.
   */
  crop_metadata?: string | null;
  /**
   * The #49 resolution ladder of the video this item hides (highest first), or
   * absent/empty when there is none.
   *
   * Present only for a video that was secured **after** its rungs were
   * generated: rung generation is gated on gallery eligibility, so securing
   * first means no ladder is ever produced. The picker therefore appears for
   * some secure videos and not others — a server-side asymmetry the client does
   * not paper over, because an empty ladder correctly draws no gear icon.
   *
   * The rung blobs belong to the hidden original, not to this item's clone, and
   * the serve path gates them behind the same unlock token this listing
   * required (`is_secure_item`'s rendition arm).
   */
  renditions?: Rendition[] | null;
};

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
    request<{ items: SecureGalleryItem[] }>(
      `/galleries/secure/${galleryId}/items`,
      {
        headers: { "X-Gallery-Token": galleryToken },
      }
    ),

  /**
   * List items across ALL of the user's secure galleries in one request.
   * Each item carries its owning `gallery_id`/`gallery_name`. Feeds the
   * built-in secure smart albums (see gallery/secureSmartAlbums.ts).
   */
  listAllItems: (galleryToken: string) =>
    request<{ items: SecureGalleryItem[] }>(`/galleries/secure/items`, {
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

  /**
   * Remove a single item from a secure album.  Deletes the encrypted clone
   * from disk and unhides the original photo so it returns to the regular
   * gallery.  See server/src/gallery/secure.rs::remove_gallery_item.
   */
  removeItem: (galleryId: string, itemId: string) =>
    request<void>(
      `/galleries/secure/${galleryId}/items/${itemId}`,
      { method: "DELETE" }
    ),

  /**
   * Move an item from one secure album to another (#31, cross-secure-album
   * picker). A photo may live in at most one secure album, so pulling media in
   * "from other secure albums" is a MOVE, not a copy — the server just reassigns
   * the membership. See server/src/gallery/secure.rs::move_gallery_item.
   */
  moveItem: (sourceGalleryId: string, itemId: string, targetGalleryId: string) =>
    request<void>(
      `/galleries/secure/${sourceGalleryId}/items/${itemId}/move`,
      {
        method: "POST",
        body: JSON.stringify({ target_gallery_id: targetGalleryId }),
      }
    ),

  /**
   * Persist (or clear, with `null`) non-destructive crop/edit metadata for a
   * secure item (#31). See server/src/gallery/secure.rs::set_gallery_item_crop.
   */
  setItemCrop: (galleryId: string, itemId: string, cropMetadata: string | null) =>
    request<{ item_id: string; crop_metadata: string | null }>(
      `/galleries/secure/${galleryId}/items/${itemId}/crop`,
      {
        method: "PUT",
        body: JSON.stringify({ crop_metadata: cropMetadata }),
      }
    ),

  /** Get all blob IDs across all secure galleries (for filtering from main gallery) */
  secureBlobIds: () =>
    request<{ blob_ids: string[] }>("/galleries/secure/blob-ids"),
};
