/**
 * Backup server API client — manage backup servers, trigger sync/recovery,
 * LAN discovery, backup mode toggle, and audio backup settings.
 *
 * Maps to server routes: `/api/admin/backup/*`, `/api/settings/audio-backup`,
 * `/api/admin/audio-backup`, `/api/admin/photos/auto-scan`.
 */
import { request } from "./core";

// ── Backup Servers API (Admin) ───────────────────────────────────────────────

export const backupApi = {
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
        /** API key returned for localhost backup-mode servers via /api/discover/info */
        api_key?: string;
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

  /** Trigger an auto-scan of the storage directory.
   *  Note: Hits `/admin/photos/auto-scan` — grouped here with backup ops
   *  since auto-scan feeds into the backup pipeline. */
  triggerAutoScan: () =>
    request<{ message: string }>("/admin/photos/auto-scan", {
      method: "POST",
    }),

  /** Get the current audio backup setting */
  getAudioBackupSetting: () =>
    request<{ audio_backup_enabled: boolean }>("/settings/audio-backup"),

  /** Set the audio backup setting (admin only) */
  setAudioBackupSetting: (enabled: boolean) =>
    request<{ audio_backup_enabled: boolean; message: string }>(
      "/admin/audio-backup",
      {
        method: "PUT",
        body: JSON.stringify({ audio_backup_enabled: enabled }),
      }
    ),
};
