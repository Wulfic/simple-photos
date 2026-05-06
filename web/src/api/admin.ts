/**
 * Admin API client — user management, storage config, server settings,
 * SSL/TLS, import scanning.
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
      smb?: {
        address: string;
        username: string;
        domain: string;
        mount_point: string;
        subpath: string;
        mounted: boolean;
      };
    }>("/admin/storage"),

  updateStorage: (path: string) =>
    request<{
      storage_path: string;
      message: string;
    }>("/admin/storage", {
      method: "PUT",
      body: JSON.stringify({ path }),
    }),

  /**
   * Configure an SMB / CIFS network share as the storage backend. The server
   * will mount the share via `mount.cifs`, point storage at the resulting
   * mount point, and persist the (encrypted) credentials so the share is
   * remounted automatically on every restart.
   */
  configureSmbStorage: (smb: {
    address: string;
    username?: string;
    password?: string;
    domain?: string;
    mount_point?: string;
  }) =>
    request<{
      storage_path: string;
      message: string;
      smb?: {
        address: string;
        username: string;
        domain: string;
        mount_point: string;
        subpath: string;
        mounted: boolean;
      };
    }>("/admin/storage", {
      method: "PUT",
      body: JSON.stringify({ smb }),
    }),

  /** Dry-run an SMB connection without mounting (for the wizard's "Test" button). */
  testSmbConnection: (smb: {
    address: string;
    username?: string;
    password?: string;
    domain?: string;
  }) =>
    request<{ ok: boolean; message: string }>("/admin/storage/test-smb", {
      method: "POST",
      body: JSON.stringify(smb),
    }),

  browseDirectory: (path?: string) =>
    request<{
      current_path: string;
      parent_path: string | null;
      directories: Array<{ name: string; path: string }>;
      writable: boolean;
    }>(`/admin/browse${path ? `?path=${encodeURIComponent(path)}` : ""}`),

  /** Open a native OS folder-picker dialog on the server machine.
   *  Returns the selected path. Throws if unavailable (headless / no zenity)
   *  or if the user cancelled. The caller should fall back to browseDirectory. */
  pickDirectory: () =>
    request<{ path: string }>("/admin/pick-directory"),

  /** Locate a sentinel file the browser wrote via showDirectoryPicker() and
   *  return the absolute server-side path of its parent directory. */
  resolveStorageSentinel: (filename: string) =>
    request<{ path: string }>(
      `/admin/resolve-sentinel?filename=${encodeURIComponent(filename)}`
    ),

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
    request<{ port: number; suggested_port: number; external_port?: number; message: string }>("/admin/port"),

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

  /** Get conversion pipeline status */
  conversionStatus: () =>
    request<{ active: boolean; total: number; done: number }>("/admin/conversion-status"),



  // ── SSL / TLS ──────────────────────────────────────────────────────────

  /** Get current TLS configuration */
  getSsl: () =>
    request<{
      enabled: boolean;
      cert_path: string | null;
      key_path: string | null;
      message: string;
      letsencrypt?: {
        domain: string;
        email: string;
        staging: boolean;
        challenge_port: number;
        last_issued_at: string | null;
      } | null;
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

  /**
   * Provision a Let's Encrypt certificate for the given domain.
   *
   * Drives the full ACME-v2 HTTP-01 handshake on the server, writes the
   * issued cert to `data/letsencrypt/{domain}/`, and persists the new
   * paths into `config.toml`.  Pass `dry_run: true` to validate inputs
   * without contacting the CA (used by the wizard to surface form errors
   * before burning a Let's Encrypt rate-limit slot).
   */
  provisionLetsEncrypt: (data: {
    domain: string;
    email: string;
    agree_tos: boolean;
    staging?: boolean;
    challenge_port?: number;
    dry_run?: boolean;
  }) =>
    request<{
      success: boolean;
      dry_run: boolean;
      domain: string;
      staging: boolean;
      cert_path: string | null;
      key_path: string | null;
      message: string;
    }>("/admin/ssl/letsencrypt", {
      method: "POST",
      body: JSON.stringify(data),
    }),

};
