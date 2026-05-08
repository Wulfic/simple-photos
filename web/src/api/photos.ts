/**
 * Photos API client — list, upload, download, favorite, crop, duplicate,
 * edit copies, and encrypted sync operations.
 *
 * Blob IDs reference encrypted data. URL builders produce authenticated
 * URLs for `<img>` / `<video>` elements that can't set headers.
 *
 * Maps to server routes: `/api/photos/*`.
 */
import { request, postRaw, BASE } from "./core";

// ── Photos API ───────────────────────────────────────────────────────────────

export const photosApi = {
  /** Get the URL for serving a photo file */
  fileUrl: (photoId: string) => `${BASE}/photos/${photoId}/file`,

  /** Get the URL for downloading the original unconverted source file */
  sourceFileUrl: (photoId: string) => `${BASE}/photos/${photoId}/source-file`,

  /** Get the URL for serving a photo thumbnail */
  thumbUrl: (photoId: string) => `${BASE}/photos/${photoId}/thumb`,

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

  /** Duplicate a photo (Save as Copy) — creates a new photos row with
   *  edits baked into a rendered file (its own encrypted blob). */
  duplicate: (photoId: string, cropMetadata: string | null) =>
    request<{
      id: string;
      source_photo_id: string;
      filename: string;
      crop_metadata: object | null;
      width: number;
      height: number;
      size_bytes: number;
      mime_type: string;
      media_type: string;
      duration_secs: number | null;
      encrypted_blob_id: string | null;
      encrypted_thumb_blob_id: string | null;
    }>(
      `/photos/${photoId}/duplicate`,
      {
        method: "POST",
        body: JSON.stringify({ crop_metadata: cropMetadata }),
      },
    ),

  /** POST /photos/:id/render — bake crop/trim/rotation/brightness into a
   *  video or audio file on the server using ffmpeg and return a Blob
   *  ready for download. cropMetadata is the JSON string from IndexedDB. */
  renderFile: (photoId: string, cropMetadata: string): Promise<Blob> =>
    postRaw(`/photos/${photoId}/render`, JSON.stringify({ crop_metadata: cropMetadata })),

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

  /** Batch-update width/height for photos (used by client-side self-heal) */
  batchUpdateDimensions: (
    updates: Array<{ photo_id?: string; blob_id?: string; width: number; height: number }>,
  ) =>
    request<{ updated: number }>("/photos/dimensions", {
      method: "PATCH",
      body: JSON.stringify({ updates }),
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
        source_path: string | null;
        photo_subtype: string | null;
        burst_id: string | null;
        motion_video_blob_id: string | null;
      }>;
      next_cursor: string | null;
    }>(`/photos/encrypted-sync${qs ? `?${qs}` : ""}`);
  },

  /** URL for serving the embedded motion video for a motion photo */
  motionVideoUrl: (photoId: string) => `${BASE}/photos/${photoId}/motion-video`,

  /** Fetch all frames in a burst group */
  burstFrames: (burstId: string) =>
    request<Array<{
      id: string;
      filename: string;
      taken_at: string | null;
      width: number;
      height: number;
      thumb_path: string | null;
    }>>(`/photos/burst/${encodeURIComponent(burstId)}`),

  /**
   * POST /photos/upload — upload a raw media file for full server-side
   * processing (EXIF/GPS extraction, server-side conversion of HEIC/MKV/etc.,
   * audio_backup_enabled enforcement, AI/geo backfill, ingest encryption).
   *
   * This is the single canonical "manual upload" path — both the gallery
   * upload button and the bulk Import page route through this endpoint so
   * that manually-added files end up in the same `photos` table as files
   * registered by the autoscan/setup-import pipeline. That guarantees
   * identical ordering, metadata, conversions, and policy enforcement.
   *
   * Optional `takenAt`, `latitude`, `longitude` overrides are forwarded
   * via headers when sidecar metadata (e.g. Google Photos Takeout JSON)
   * supplies values the file's EXIF lacks.
   */
  upload: async (
    data: ArrayBuffer,
    filename: string,
    mimeType: string,
    overrides?: {
      takenAt?: string;
      latitude?: number;
      longitude?: number;
      /**
       * File's last-modified timestamp in epoch milliseconds (browser
       * `File.lastModified`). Used as a fallback for `taken_at` when EXIF
       * and any explicit takenAt sidecar value are absent — mirrors the
       * autoscan pipeline's behaviour of preferring file mtime over "now",
       * so uploads land in the correct timeline slot rather than at the top.
       */
      fileModifiedAt?: number;
    },
  ): Promise<{
    photo_id: string;
    filename: string;
    file_path: string;
    size_bytes: number;
    photo_hash: string | null;
  }> => {
    const headers: Record<string, string> = {
      "X-Filename": filename,
      "X-Mime-Type": mimeType,
    };
    if (overrides?.takenAt) headers["X-Taken-At"] = overrides.takenAt;
    if (typeof overrides?.latitude === "number") {
      headers["X-Latitude"] = overrides.latitude.toString();
    }
    if (typeof overrides?.longitude === "number") {
      headers["X-Longitude"] = overrides.longitude.toString();
    }
    if (
      typeof overrides?.fileModifiedAt === "number" &&
      Number.isFinite(overrides.fileModifiedAt) &&
      overrides.fileModifiedAt > 0
    ) {
      headers["X-File-Modified-At"] = Math.floor(overrides.fileModifiedAt).toString();
    }
    return request("/photos/upload", {
      method: "POST",
      headers,
      body: data,
    });
  },

};
