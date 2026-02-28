import { request, downloadRaw, BASE } from "./core";
import { adminApi } from "./admin";
import type {
  RegisterResponse,
  LoginResponse,
  TotpSetupResponse,
  ChangePasswordResponse,
  DiagnosticsResponse,
  AuditLogListResponse,
  AuditLogParams,
  ClientLogListResponse,
  ClientLogParams,
} from "./types";

// Re-export all types for backward compatibility
export type {
  RegisterResponse,
  LoginResponse,
  TotpSetupResponse,
  ChangePasswordResponse,
  DiagnosticsResponse,
  AuditLogEntry,
  AuditLogListResponse,
  AuditLogParams,
  ClientLogEntry,
  ClientLogListResponse,
  ClientLogParams,
} from "./types";

// ── Public API ───────────────────────────────────────────────────────────────

export const api = {
  auth: {
    register: (username: string, password: string) =>
      request<RegisterResponse>("/auth/register", {
        method: "POST",
        body: JSON.stringify({ username, password }),
      }),

    login: (username: string, password: string) =>
      request<LoginResponse>("/auth/login", {
        method: "POST",
        body: JSON.stringify({ username, password }),
      }),

    loginTotp: (
      totp_session_token: string,
      totp_code?: string,
      backup_code?: string
    ) =>
      request<{
        access_token: string;
        refresh_token: string;
        expires_in: number;
      }>("/auth/login/totp", {
        method: "POST",
        body: JSON.stringify({ totp_session_token, totp_code, backup_code }),
      }),

    refresh: (refresh_token: string) =>
      request<{
        access_token: string;
        refresh_token: string;
        expires_in: number;
      }>("/auth/refresh", {
        method: "POST",
        body: JSON.stringify({ refresh_token }),
      }),

    logout: (refresh_token: string) =>
      request<void>("/auth/logout", {
        method: "POST",
        body: JSON.stringify({ refresh_token }),
      }),

    changePassword: (currentPassword: string, newPassword: string) =>
      request<ChangePasswordResponse>("/auth/password", {
        method: "PUT",
        body: JSON.stringify({
          current_password: currentPassword,
          new_password: newPassword,
        }),
      }),

    setup2fa: () =>
      request<TotpSetupResponse>("/auth/2fa/setup", { method: "POST" }),

    confirm2fa: (totp_code: string) =>
      request<void>("/auth/2fa/confirm", {
        method: "POST",
        body: JSON.stringify({ totp_code }),
      }),

    disable2fa: (totp_code: string) =>
      request<void>("/auth/2fa/disable", {
        method: "POST",
        body: JSON.stringify({ totp_code }),
      }),
  },

  blobs: {
    upload: (data: ArrayBuffer, blobType: string, clientHash?: string) => {
      const headers: Record<string, string> = {
        "X-Blob-Type": blobType,
        "X-Blob-Size": data.byteLength.toString(),
      };
      if (clientHash) headers["X-Client-Hash"] = clientHash;

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
        }>;
        next_cursor: string | null;
      }>(`/blobs${qs ? `?${qs}` : ""}`);
    },

    download: async (blobId: string): Promise<ArrayBuffer> => {
      return downloadRaw(`${BASE}/blobs/${blobId}`);
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
  },

  admin: adminApi,

  // ── Plain-mode Photos ─────────────────────────────────────────────────────

  photos: {
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

    /** Get the URL for serving a plain photo file */
    fileUrl: (photoId: string) => `${BASE}/photos/${photoId}/file`,

    /** Get the URL for serving a plain photo thumbnail */
    thumbUrl: (photoId: string) => `${BASE}/photos/${photoId}/thumb`,

    /** Download a plain photo file as raw binary, with 401 refresh retry */
    downloadFile: (photoId: string) =>
      downloadRaw(`${BASE}/photos/${photoId}/file`),

    /** Download a plain photo thumbnail as raw binary, with 401 refresh retry */
    downloadThumb: (photoId: string) =>
      downloadRaw(`${BASE}/photos/${photoId}/thumb`),

    delete: (photoId: string) =>
      request<void>(`/photos/${photoId}`, { method: "DELETE" }),

    /** Mark a plain photo as encrypted by linking it to the uploaded blob */
    markEncrypted: (photoId: string, blobId: string) =>
      request<{ ok: boolean }>(`/photos/${photoId}/mark-encrypted`, {
        method: "POST",
        body: JSON.stringify({ blob_id: blobId }),
      }),

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
  },

  // ── Encryption Settings ───────────────────────────────────────────────────

  encryption: {
    getSettings: () =>
      request<{
        encryption_mode: string;
        migration_status: string;
        migration_total: number;
        migration_completed: number;
        migration_error: string | null;
      }>("/settings/encryption"),

    setMode: (mode: "plain" | "encrypted") =>
      request<{ message: string; mode: string; migration_items: number }>(
        "/admin/encryption",
        {
          method: "PUT",
          body: JSON.stringify({ mode }),
        }
      ),

    reportProgress: (data: {
      completed_count: number;
      error?: string;
      done?: boolean;
    }) =>
      request<{ ok: boolean }>("/admin/encryption/progress", {
        method: "POST",
        body: JSON.stringify(data),
      }),
  },

  // ── Secure Galleries ─────────────────────────────────────────────────────

  secureGalleries: {
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
        items: Array<{ id: string; blob_id: string; added_at: string }>;
      }>(`/galleries/secure/${galleryId}/items`, {
        headers: { "X-Gallery-Token": galleryToken },
      }),

    addItem: (galleryId: string, blobId: string) =>
      request<{ item_id: string }>(
        `/galleries/secure/${galleryId}/items`,
        {
          method: "POST",
          body: JSON.stringify({ blob_id: blobId }),
        }
      ),

    /** Get all blob IDs across all secure galleries (for filtering from main gallery) */
    secureBlobIds: () =>
      request<{ blob_ids: string[] }>("/galleries/secure/blob-ids"),
  },

  // ── Storage Stats ─────────────────────────────────────────────────────────

  storageStats: {
    get: () =>
      request<{
        photo_bytes: number;
        photo_count: number;
        video_bytes: number;
        video_count: number;
        other_blob_bytes: number;
        other_blob_count: number;
        plain_bytes: number;
        plain_count: number;
        user_total_bytes: number;
        fs_total_bytes: number;
        fs_free_bytes: number;
      }>("/settings/storage-stats"),
  },

  // ── Trash ─────────────────────────────────────────────────────────────────

  trash: {
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
  },

  // ── Backup Servers (Admin) ────────────────────────────────────────────────

  backup: {
    /** List all configured backup servers */
    listServers: () =>
      request<{
        servers: Array<{
          id: string;
          name: string;
          address: string;
          sync_frequency_hours: number;
          last_sync_at: string | null;
          last_sync_status: string;
          last_sync_error: string | null;
          enabled: boolean;
          created_at: string;
        }>;
      }>("/admin/backup/servers"),

    /** Add a new backup server */
    addServer: (data: {
      name: string;
      address: string;
      api_key?: string;
      sync_frequency_hours?: number;
    }) =>
      request<{
        id: string;
        name: string;
        address: string;
        sync_frequency_hours: number;
      }>("/admin/backup/servers", {
        method: "POST",
        body: JSON.stringify(data),
      }),

    /** Update a backup server's configuration */
    updateServer: (
      serverId: string,
      data: {
        name?: string;
        address?: string;
        api_key?: string;
        sync_frequency_hours?: number;
        enabled?: boolean;
      }
    ) =>
      request<{ message: string; id: string }>(
        `/admin/backup/servers/${serverId}`,
        {
          method: "PUT",
          body: JSON.stringify(data),
        }
      ),

    /** Remove a backup server */
    removeServer: (serverId: string) =>
      request<void>(`/admin/backup/servers/${serverId}`, {
        method: "DELETE",
      }),

    /** Check if a backup server is reachable */
    checkStatus: (serverId: string) =>
      request<{
        reachable: boolean;
        version: string | null;
        error: string | null;
      }>(`/admin/backup/servers/${serverId}/status`),

    /** Get sync logs for a backup server */
    getSyncLogs: (serverId: string) =>
      request<
        Array<{
          id: string;
          server_id: string;
          started_at: string;
          completed_at: string | null;
          status: string;
          photos_synced: number;
          bytes_synced: number;
          error: string | null;
        }>
      >(`/admin/backup/servers/${serverId}/logs`),

    /** Trigger an immediate sync to a backup server */
    triggerSync: (serverId: string) =>
      request<{ message: string; sync_id: string }>(
        `/admin/backup/servers/${serverId}/sync`,
        { method: "POST" }
      ),

    /** Discover Simple Photos servers on the local network */
    discover: () =>
      request<{
        servers: Array<{
          address: string;
          name: string;
          version: string;
        }>;
      }>("/admin/backup/discover"),

    /** List photos on a backup server (proxy) */
    listBackupPhotos: (serverId: string) =>
      request<
        Array<{
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
          thumb_path: string | null;
          created_at: string;
        }>
      >(`/admin/backup/servers/${serverId}/photos`),

    /** Trigger recovery from a backup server (downloads all missing photos) */
    recover: (serverId: string) =>
      request<{ message: string; recovery_id: string }>(
        `/admin/backup/servers/${serverId}/recover`,
        { method: "POST" }
      ),

    /** Get the current backup mode and server IP */
    getMode: () =>
      request<{
        mode: string;
        server_ip: string;
        server_address: string;
        port: number;
      }>("/admin/backup/mode"),

    /** Set backup mode ("primary" or "backup") */
    setMode: (mode: string) =>
      request<{
        mode: string;
        server_ip: string;
        server_address: string;
        port: number;
      }>("/admin/backup/mode", {
        method: "POST",
        body: JSON.stringify({ mode }),
      }),

    /** Trigger an auto-scan of the storage directory */
    triggerAutoScan: () =>
      request<{ message: string }>("/admin/photos/auto-scan", {
        method: "POST",
      }),
  },

  // ── Shared Albums ───────────────────────────────────────────────────────

  sharing: {
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
  },

  // ── Tags ────────────────────────────────────────────────────────────────

  tags: {
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
  },

  // ── Search ──────────────────────────────────────────────────────────────

  search: {
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
  },

  // ── Diagnostics (admin) ─────────────────────────────────────────────────

  diagnostics: {
    /** Get comprehensive server metrics (admin only) */
    getMetrics: () =>
      request<DiagnosticsResponse>("/admin/diagnostics"),

    /** List audit log entries with optional filters (admin only) */
    listAuditLogs: (params?: AuditLogParams) => {
      const search = new URLSearchParams();
      if (params?.event_type) search.set("event_type", params.event_type);
      if (params?.user_id) search.set("user_id", params.user_id);
      if (params?.ip_address) search.set("ip_address", params.ip_address);
      if (params?.after) search.set("after", params.after);
      if (params?.before) search.set("before", params.before);
      if (params?.limit) search.set("limit", params.limit.toString());
      const qs = search.toString();
      return request<AuditLogListResponse>(
        `/admin/audit-logs${qs ? `?${qs}` : ""}`
      );
    },

    /** List client diagnostic logs with optional filters (admin only) */
    listClientLogs: (params?: ClientLogParams) => {
      const search = new URLSearchParams();
      if (params?.user_id) search.set("user_id", params.user_id);
      if (params?.session_id) search.set("session_id", params.session_id);
      if (params?.level) search.set("level", params.level);
      if (params?.after) search.set("after", params.after);
      if (params?.limit) search.set("limit", params.limit.toString());
      const qs = search.toString();
      return request<ClientLogListResponse>(
        `/admin/client-logs${qs ? `?${qs}` : ""}`
      );
    },
  },
};
