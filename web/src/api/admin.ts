/**
 * Admin API client — user management, storage config, server settings,
 * SSL/TLS, import scanning, media conversion triggers.
 *
 * All endpoints require admin role (enforced server-side).
 * Maps to server routes: `/api/admin/*` plus a few `/api/photos/` helpers.
 */
import { request, tryRefresh, BASE } from "./core";
import { useAuthStore } from "../store/auth";
import type { TotpSetupResponse } from "./types";

// ── Admin API ────────────────────────────────────────────────────────────────

export const adminApi = {
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

  deleteUser: (userId: string) =>
    request<void>(`/admin/users/${userId}`, { method: "DELETE" }),

  updateUserRole: (userId: string, role: "admin" | "user") =>
    request<{ message: string; user_id: string; role: string }>(
      `/admin/users/${userId}/role`,
      {
        method: "PUT",
        body: JSON.stringify({ role }),
      }
    ),

  resetUserPassword: (userId: string, newPassword: string) =>
    request<{ message: string }>(
      `/admin/users/${userId}/password`,
      {
        method: "PUT",
        body: JSON.stringify({ new_password: newPassword }),
      }
    ),

  resetUser2fa: (userId: string) =>
    request<{ message: string }>(`/admin/users/${userId}/2fa`, {
      method: "DELETE",
    }),

  setupUser2fa: (userId: string) =>
    request<TotpSetupResponse>(`/admin/users/${userId}/2fa/setup`, {
      method: "POST",
    }),

  confirmUser2fa: (userId: string, totpCode: string) =>
    request<{ message: string }>(`/admin/users/${userId}/2fa/confirm`, {
      method: "POST",
      body: JSON.stringify({ totp_code: totpCode }),
    }),

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

  /** Scan storage and register all unregistered media files */
  scanAndRegister: () =>
    request<{ registered: number; message: string }>("/admin/photos/scan", {
      method: "POST",
    }),

  /** Trigger an immediate background conversion pass (thumbnails + web previews) */
  triggerConvert: () =>
    request<{ message: string }>("/admin/photos/convert", {
      method: "POST",
    }),

  /** Supply encryption key and trigger re-conversion of encrypted blobs
   *  that still contain raw (non-web-compatible) media data. */
  triggerReconvert: (keyHex: string) =>
    request<{ message: string; needs_conversion: number }>(
      "/admin/photos/reconvert",
      {
        method: "POST",
        body: JSON.stringify({ key_hex: keyHex }),
      }
    ),

  /** Check how many files still need conversion or thumbnails.
   *  Note: Hits `/photos/conversion-status` (not `/admin/...`) — lives here
   *  for UI grouping since only admins use it in practice. */
  conversionStatus: () =>
    request<{
      pending_conversions: number;
      pending_awaiting_key: number;
      missing_thumbnails: number;
      converting: boolean;
      key_available?: boolean;
    }>(
      "/photos/conversion-status"
    ),

  // ── SSL / TLS ──────────────────────────────────────────────────────────

  /** Get current TLS configuration */
  getSsl: () =>
    request<{
      enabled: boolean;
      cert_path: string | null;
      key_path: string | null;
      message: string;
    }>("/admin/ssl"),

  /** Update TLS configuration (manual cert paths) */
  updateSsl: (data: {
    enabled: boolean;
    cert_path?: string;
    key_path?: string;
  }) =>
    request<{
      enabled: boolean;
      cert_path: string | null;
      key_path: string | null;
      message: string;
    }>("/admin/ssl", {
      method: "PUT",
      body: JSON.stringify(data),
    }),

};
