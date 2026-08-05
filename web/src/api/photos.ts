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
import type { Rendition } from "../gallery/renditionChoice";

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
  /**
   * Photo metadata for encrypted-mode sync.
   *
   * Two modes. Without `since`, the full keyset walk over the whole eligible
   * library — self-healing, because the client set-differences what it receives
   * against what it holds. With `since`, only what changed after that
   * change-log sequence, plus `deleted` tombstones naming everything that left
   * the feed (#38).
   *
   * `since: 0` is not a special case — migration 033 backfilled a change-log
   * row for every pre-existing photo, so it degenerates into a full enumeration.
   */
  encryptedSync: (params?: { after?: string; limit?: number; since?: number }) => {
    const query = new URLSearchParams();
    if (params?.after) query.set("after", params.after);
    if (params?.limit) query.set("limit", params.limit.toString());
    // `since: 0` is meaningful, so test for undefined rather than falsiness.
    if (params?.since !== undefined) query.set("since", params.since.toString());
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
        /**
         * Playable qualities for this video, highest first (#49).
         *
         * Optional because a pre-#49 server omits the field entirely — and
         * because an **empty array is the normal case**: only videos above the
         * 1080p tier ever get a second rung, so most records carry nothing here
         * and the viewer must draw no picker rather than an empty one.
         */
        renditions?: Rendition[];
      }>;
      next_cursor: string | null;
      /**
       * Ids of photos that have left the feed — deleted outright, or claimed by
       * a secure gallery. The client treats both identically: drop the row.
       *
       * **Present (possibly empty) on every delta response; absent on a full
       * walk.** That distinction is the protocol handshake, not a detail: a
       * server too old to know `since` ignores the parameter and answers with a
       * full walk, which is indistinguishable from a delta by its `photos` alone.
       * Reading `deleted === undefined` as "nothing was removed" would make the
       * client prune nothing and accumulate ghost rows forever. `syncPass`
       * therefore treats an absent `deleted` as "this server does not speak
       * delta" and falls back to the full walk.
       */
      deleted?: string[];
      /** Change-log head at the time this page was built. Persist the value
       *  from the FIRST page of a walk — see `syncPass.ts` for why the last
       *  page's head loses concurrent writes. */
      head_seq: number;
    }>(`/photos/encrypted-sync${qs ? `?${qs}` : ""}`);
  },

  /** Cheap precomputed gallery counts (server-side aggregate, TTL-cached).
   *  This is the **authoritative** source for smart-album badges (#42): the
   *  local IndexedDB mirror only holds rows that carry an encrypted blob, so
   *  counting it under-reports the library by however many photos are still
   *  awaiting client-side encryption.
   *
   *  Two families of number, NOT interchangeable — see `PhotoSummary` in
   *  `server/src/gallery/summary.rs`:
   *  - `total`/`photos`/`gifs`/... are raw media-type ROW counts.
   *  - `smart_*` are TILE counts: the client's smart-album filter applied
   *    first, burst frames collapsed second. Badges must use these. */
  summary: () =>
    request<{
      total: number;
      collapsed_total: number;
      photos: number;
      gifs: number;
      videos: number;
      audio: number;
      favorites: number;
      smart_photos: number;
      smart_gifs: number;
      smart_videos: number;
      smart_audio: number;
      smart_favorites: number;
      smart_recent: number;
      /** Change-log head (#38) — compare against the last synced sequence to
       *  decide whether `encrypted-sync` needs to run at all. */
      head_seq: number;
    }>("/photos/summary"),

  /** Authoritative `album_name → [photo_id]` mapping captured at Takeout import
   *  time (server-side, keyed by photo id — survives filename collisions and
   *  `-edited` dedup). Used to rebuild album manifests deterministically.
   *
   *  `name` is the Takeout *folder* name and is the album's identity (the
   *  deterministic album id derives from it), so it stays stable even though
   *  Google mangles it. `title` is the album's real Google Photos name, read
   *  from the album's `metadata.json`; display it in preference to `name`, and
   *  fall back to `name` when it's null (older exports don't carry one). */
  sourceAlbums: () =>
    request<{
      albums: Array<{
        name: string;
        title: string | null;
        source: string;
        photo_ids: string[];
      }>;
    }>("/photos/source-albums"),

  /** Tombstone a Takeout-reconstructed album the user deleted, so reconstruction
   *  stops recreating it on this and every other device. `dismissed: false` means
   *  the id wasn't a source album at all (an ordinary user album) — not an error.
   *  Identified by the local album id; the server resolves it back to the album
   *  identity by recomputing the same hash. Photos are not affected. */
  dismissSourceAlbum: (albumId: string) =>
    request<{ dismissed: boolean; name: string | null }>(
      "/photos/source-albums/dismiss",
      { method: "POST", body: JSON.stringify({ album_id: albumId }) },
    ),

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
      /**
       * Ask the server to defer conversion of non-native formats (HEIC, MKV,
       * …) to its background pass instead of converting inline. Used by the
       * bulk Import page so one slow FFmpeg run can't freeze the sequential
       * upload loop. The server only honors this for convertible files from
       * an admin with no metadata overrides; everything else converts inline
       * as before. A deferred upload returns `{ status: "queued" }` (202)
       * instead of a photo record.
       */
      deferConversion?: boolean;
      /**
       * Takeout album folder this file was picked from, derived client-side from
       * the folder structure (`utils/uploadAlbums.ts`). Recorded server-side in
       * `photo_source_albums` so the album can be rebuilt — without it, a Takeout
       * imported through the browser loses all its album data. Note the server
       * will not defer conversion for an upload carrying this.
       */
      sourceAlbum?: string;
      /** The album's real Google Photos title from its `metadata.json`. */
      sourceAlbumTitle?: string;
    },
  ): Promise<
    | {
        photo_id: string;
        filename: string;
        file_path: string;
        size_bytes: number;
        photo_hash: string | null;
      }
    | { status: "queued"; filename: string; deferred: true }
  > => {
    const headers: Record<string, string> = {
      "X-Filename": filename,
      "X-Mime-Type": mimeType,
    };
    if (overrides?.deferConversion) headers["X-Defer-Conversion"] = "1";
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
    // Percent-encoded: header values are bytes, and `fetch` throws outright on a
    // non-Latin-1 string — an album called "東京 2019" would fail the upload.
    // The server percent-decodes and sanitises (see photos/upload.rs).
    if (overrides?.sourceAlbum) {
      headers["X-Source-Album"] = encodeURIComponent(overrides.sourceAlbum);
    }
    if (overrides?.sourceAlbumTitle) {
      headers["X-Source-Album-Title"] = encodeURIComponent(
        overrides.sourceAlbumTitle,
      );
    }
    return request("/photos/upload", {
      method: "POST",
      headers,
      body: data,
    });
  },

};
