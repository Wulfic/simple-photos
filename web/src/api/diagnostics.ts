import { request } from "./core";
import type {
  DiagnosticsResponseUnion,
  DiagnosticsConfig,
  UpdateDiagnosticsConfigRequest,
  AuditLogListResponse,
  AuditLogParams,
  ClientLogListResponse,
  ClientLogParams,
} from "./types";

// ── Storage Stats API ────────────────────────────────────────────────────────

export const storageStatsApi = {
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
};

// ── Diagnostics API (admin) ──────────────────────────────────────────────────

export const diagnosticsApi = {
  /** Get comprehensive server metrics (admin only). Returns lightweight stub when disabled. */
  getMetrics: () =>
    request<DiagnosticsResponseUnion>("/admin/diagnostics"),

  /** Get diagnostics configuration (admin only) */
  getConfig: () =>
    request<DiagnosticsConfig>("/admin/diagnostics/config"),

  /** Update diagnostics configuration (admin only) */
  updateConfig: (config: UpdateDiagnosticsConfigRequest) =>
    request<DiagnosticsConfig>("/admin/diagnostics/config", {
      method: "PUT",
      body: JSON.stringify(config),
    }),

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
};
