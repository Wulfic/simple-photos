import { useAuthStore } from "../store/auth";

const BASE = "/api";

/**
 * Centralized, security-hardened API client.
 *
 * Security features:
 * - X-Requested-With header on all requests (basic CSRF protection)
 * - Full refresh-token rotation (server returns new refresh token on each refresh)
 * - Automatic token refresh on 401, with single-flight deduplication
 * - Rate-limit aware: surfaces 429 messages to the user
 * - Rejects blobs with non-2xx status even on download
 * - Never logs tokens
 */

// ── Single-flight refresh deduplication ──────────────────────────────────────
// If multiple requests 401 at the same time, only one refresh attempt runs.
let refreshPromise: Promise<boolean> | null = null;

async function request<T>(
  path: string,
  options: RequestInit = {}
): Promise<T> {
  const { accessToken } = useAuthStore.getState();

  const headers: Record<string, string> = {
    ...(options.headers as Record<string, string>),
    // Basic CSRF protection — server can reject requests without this header
    "X-Requested-With": "SimplePhotos",
  };

  if (accessToken) {
    headers["Authorization"] = `Bearer ${accessToken}`;
  }

  // Only set Content-Type for JSON bodies (not raw blob uploads)
  if (options.body && typeof options.body === "string") {
    headers["Content-Type"] = "application/json";
  }

  const res = await fetch(`${BASE}${path}`, { ...options, headers });

  // ── Rate limiting ────────────────────────────────────────────────────────
  if (res.status === 429) {
    const retryAfter = res.headers.get("Retry-After");
    const err = await res.json().catch(() => ({ error: "Too many requests" }));
    const msg = retryAfter
      ? `${err.error || "Too many requests"}. Try again in ${retryAfter}s.`
      : err.error || "Too many requests. Please wait and try again.";
    throw new Error(msg);
  }

  // ── Automatic token refresh on 401 ──────────────────────────────────────
  // Skip refresh logic for auth endpoints — their 401s are real errors
  // (wrong password, invalid token), not expired-session indicators.
  const isAuthEndpoint = path.startsWith("/auth/");
  if (res.status === 401 && !isAuthEndpoint) {
    const refreshed = await tryRefresh();
    if (refreshed) {
      const newToken = useAuthStore.getState().accessToken;
      headers["Authorization"] = `Bearer ${newToken}`;
      const retry = await fetch(`${BASE}${path}`, { ...options, headers });

      if (retry.status === 429) {
        throw new Error("Too many requests. Please wait and try again.");
      }
      if (!retry.ok) {
        const err = await retry.json().catch(() => ({ error: "Request failed" }));
        throw new Error(err.error || `HTTP ${retry.status}`);
      }
      if (retry.status === 204) return undefined as T;
      const retryText = await retry.text();
      if (!retryText) return undefined as T;
      return JSON.parse(retryText) as T;
    }
    // Refresh failed — force logout
    useAuthStore.getState().logout();
    throw new Error("Session expired. Please sign in again.");
  }

  if (!res.ok) {
    const err = await res.json().catch(() => ({ error: "Request failed" }));
    throw new Error(err.error || `HTTP ${res.status}`);
  }

  if (res.status === 204) return undefined as T;

  // Handle empty response bodies (e.g. 200 OK with no content)
  const text = await res.text();
  if (!text) return undefined as T;
  return JSON.parse(text) as T;
}

/**
 * Attempt to refresh the access token.
 *
 * Supports full token rotation: the server returns a NEW refresh token
 * alongside the new access token. Both are persisted.
 *
 * Uses single-flight deduplication so concurrent 401s don't cause
 * multiple refresh attempts (which would fail with revoked-token detection).
 */
async function tryRefresh(): Promise<boolean> {
  // If a refresh is already in flight, piggyback on it
  if (refreshPromise) return refreshPromise;

  refreshPromise = (async () => {
    const { refreshToken, setTokens } = useAuthStore.getState();
    if (!refreshToken) return false;

    try {
      const res = await fetch(`${BASE}/auth/refresh`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-Requested-With": "SimplePhotos",
        },
        body: JSON.stringify({ refresh_token: refreshToken }),
      });
      if (!res.ok) return false;

      const data = await res.json();
      // Server returns rotated refresh token — persist both new tokens
      const newRefresh = data.refresh_token || refreshToken;
      setTokens(data.access_token, newRefresh);
      return true;
    } catch {
      return false;
    }
  })();

  try {
    return await refreshPromise;
  } finally {
    refreshPromise = null;
  }
}

// ── Type definitions ─────────────────────────────────────────────────────────

export interface RegisterResponse {
  user_id: string;
  username: string;
}

export interface LoginResponse {
  access_token?: string;
  refresh_token?: string;
  expires_in?: number;
  requires_totp?: boolean;
  totp_session_token?: string;
}

export interface TotpSetupResponse {
  otpauth_uri: string;
  backup_codes: string[];
}

export interface ChangePasswordResponse {
  message: string;
}

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
      const { accessToken } = useAuthStore.getState();
      const headers: Record<string, string> = {
        "X-Requested-With": "SimplePhotos",
      };
      if (accessToken) {
        headers["Authorization"] = `Bearer ${accessToken}`;
      }

      const res = await fetch(`${BASE}/blobs/${blobId}`, { headers });

      if (res.status === 401) {
        // Try one refresh then retry
        const refreshed = await tryRefresh();
        if (refreshed) {
          const newToken = useAuthStore.getState().accessToken;
          headers["Authorization"] = `Bearer ${newToken}`;
          const retry = await fetch(`${BASE}/blobs/${blobId}`, { headers });
          if (!retry.ok) throw new Error(`Download failed: ${retry.status}`);
          return retry.arrayBuffer();
        }
        useAuthStore.getState().logout();
        throw new Error("Session expired");
      }

      if (!res.ok) throw new Error(`Failed to download blob: ${res.status}`);
      return res.arrayBuffer();
    },

    delete: (blobId: string) =>
      request<void>(`/blobs/${blobId}`, { method: "DELETE" }),
  },

  admin: {
    createUser: (username: string, password: string, role: "admin" | "user" = "user") =>
      request<{
        user_id: string;
        username: string;
        role: string;
      }>("/admin/users", {
        method: "POST",
        body: JSON.stringify({ username, password, role }),
      }),

    listUsers: () =>
      request<
        Array<{
          id: string;
          username: string;
          role: string;
          totp_enabled: boolean;
          created_at: string;
        }>
      >("/admin/users"),

    getStorage: () =>
      request<{
        storage_path: string;
        message: string;
      }>("/admin/storage"),

    updateStorage: (path: string) =>
      request<{
        storage_path: string;
        message: string;
      }>("/admin/storage", {
        method: "PUT",
        body: JSON.stringify({ path }),
      }),

    browseDirectory: (path?: string) =>
      request<{
        current_path: string;
        parent_path: string | null;
        directories: Array<{ name: string; path: string }>;
        writable: boolean;
      }>(`/admin/browse${path ? `?path=${encodeURIComponent(path)}` : ""}`),

    importScan: (path?: string) =>
      request<{
        directory: string;
        files: Array<{
          name: string;
          path: string;
          size: number;
          mime_type: string;
          modified: string | null;
        }>;
        total_size: number;
      }>(`/admin/import/scan${path ? `?path=${encodeURIComponent(path)}` : ""}`),

    importFile: async (filePath: string): Promise<ArrayBuffer> => {
      const { accessToken } = useAuthStore.getState();
      const headers: Record<string, string> = {
        "X-Requested-With": "SimplePhotos",
      };
      if (accessToken) {
        headers["Authorization"] = `Bearer ${accessToken}`;
      }

      const url = `${BASE}/admin/import/file?path=${encodeURIComponent(filePath)}`;
      const res = await fetch(url, { headers });

      if (res.status === 401) {
        const refreshed = await tryRefresh();
        if (refreshed) {
          const newToken = useAuthStore.getState().accessToken;
          headers["Authorization"] = `Bearer ${newToken}`;
          const retry = await fetch(url, { headers });
          if (!retry.ok) throw new Error(`Download failed: ${retry.status}`);
          return retry.arrayBuffer();
        }
        useAuthStore.getState().logout();
        throw new Error("Session expired");
      }

      if (!res.ok) {
        const err = await res.json().catch(() => ({ error: `HTTP ${res.status}` }));
        throw new Error(err.error || `HTTP ${res.status}`);
      }
      return res.arrayBuffer();
    },

    getPort: () =>
      request<{ port: number; message: string }>("/admin/port"),

    updatePort: (port: number) =>
      request<{ port: number; message: string }>("/admin/port", {
        method: "PUT",
        body: JSON.stringify({ port }),
      }),

    restart: () =>
      request<{ message: string }>("/admin/restart", {
        method: "POST",
      }),

    /** Scan storage and register all unregistered media files (plain mode) */
    scanAndRegister: () =>
      request<{ registered: number; message: string }>("/admin/photos/scan", {
        method: "POST",
      }),
  },

  // ── Plain-mode Photos ─────────────────────────────────────────────────────

  photos: {
    list: (params?: { after?: string; limit?: number; media_type?: string }) => {
      const query = new URLSearchParams();
      if (params?.after) query.set("after", params.after);
      if (params?.limit) query.set("limit", params.limit.toString());
      if (params?.media_type) query.set("media_type", params.media_type);
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

    delete: (photoId: string) =>
      request<void>(`/photos/${photoId}`, { method: "DELETE" }),
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

  // ── Encrypted Galleries ───────────────────────────────────────────────────

  encryptedGalleries: {
    list: () =>
      request<{
        galleries: Array<{
          id: string;
          name: string;
          created_at: string;
          item_count: number;
        }>;
      }>("/galleries/encrypted"),

    create: (name: string, password: string) =>
      request<{ gallery_id: string; name: string }>("/galleries/encrypted", {
        method: "POST",
        body: JSON.stringify({ name, password }),
      }),

    delete: (galleryId: string) =>
      request<void>(`/galleries/encrypted/${galleryId}`, {
        method: "DELETE",
      }),

    unlock: (galleryId: string, password: string) =>
      request<{ gallery_token: string; expires_in: number }>(
        `/galleries/encrypted/${galleryId}/unlock`,
        {
          method: "POST",
          body: JSON.stringify({ password }),
        }
      ),

    listItems: (galleryId: string, galleryToken: string) =>
      request<{
        items: Array<{ id: string; blob_id: string; added_at: string }>;
      }>(`/galleries/encrypted/${galleryId}/items`, {
        headers: { "X-Gallery-Token": galleryToken },
      }),

    addItem: (galleryId: string, blobId: string) =>
      request<{ item_id: string }>(
        `/galleries/encrypted/${galleryId}/items`,
        {
          method: "POST",
          body: JSON.stringify({ blob_id: blobId }),
        }
      ),
  },
};
