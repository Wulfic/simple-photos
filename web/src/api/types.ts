/**
 * Shared TypeScript interfaces for API request/response payloads.
 *
 * These types mirror the server-side Rust DTOs and are used by both the
 * API client functions and UI components to ensure type safety across the
 * network boundary.
 */

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

// ── Diagnostics types ────────────────────────────────────────────────────

/** Server-side diagnostics configuration (admin controls) */
export interface DiagnosticsConfig {
  diagnostics_enabled: boolean;
  client_diagnostics_enabled: boolean;
}

/** Request body for updating diagnostics configuration */
export interface UpdateDiagnosticsConfigRequest {
  diagnostics_enabled?: boolean;
  client_diagnostics_enabled?: boolean;
}

/** Lightweight response when diagnostics collection is disabled */
export interface DisabledDiagnosticsResponse {
  enabled: false;
  server: {
    version: string;
    uptime_seconds: number;
    started_at: string;
  };
  message: string;
}

export interface DiagnosticsResponse {
  enabled: true;
  server: {
    version: string;
    uptime_seconds: number;
    rust_version: string;
    os: string;
    arch: string;
    memory_rss_bytes: number;
    cpu_seconds: number;
    pid: number;
    storage_root: string;
    db_path: string;
    tls_enabled: boolean;
    max_blob_size_mb: number;
    started_at: string;
    thread_count: number;
    open_fds: number;
    load_average: [number, number, number];
  };
  database: {
    size_bytes: number;
    wal_size_bytes: number;
    table_counts: Record<string, number>;
    journal_mode: string;
    page_size: number;
    page_count: number;
    freelist_count: number;
  };
  storage: {
    total_bytes: number;
    file_count: number;
    disk_total_bytes: number;
    disk_available_bytes: number;
    disk_used_percent: number;
  };
  users: {
    total_users: number;
    admin_count: number;
    totp_enabled_count: number;
  };
  photos: {
    total_photos: number;
    encrypted_count: number;
    total_file_bytes: number;
    total_thumb_bytes: number;
    photos_with_thumbs: number;
    photos_by_media_type: Record<string, number>;
    oldest_photo: string | null;
    newest_photo: string | null;
    favorited_count: number;
    tagged_count: number;
  };
  audit: {
    total_entries: number;
    entries_last_24h: number;
    entries_last_7d: number;
    events_by_type: Record<string, number>;
    recent_failures: Array<{
      event_type: string;
      ip_address: string;
      user_agent: string;
      created_at: string;
      details: string;
    }>;
  };
  client_logs: {
    total_entries: number;
    entries_last_24h: number;
    entries_last_7d: number;
    by_level: Record<string, number>;
    unique_sessions: number;
  };
  backup: {
    server_count: number;
    total_sync_logs: number;
    last_sync_at: string | null;
    servers: Array<{
      id: string;
      name: string;
      address: string;
      enabled: boolean;
      sync_frequency_hours: number;
      last_sync_at: string | null;
      last_sync_status: string;
      last_sync_error: string | null;
      last_diagnostics: {
        version?: string;
        uptime_seconds?: number;
        memory_rss_bytes?: number;
        cpu_seconds?: number;
        total_photos?: number;
        disk_used_percent?: number;
        db_size_bytes?: number;
        collected_at?: string;
      } | null;
      last_diagnostics_at: string | null;
      recent_sync_logs: Array<{
        id: string;
        started_at: string;
        completed_at: string | null;
        status: string;
        photos_synced: number;
        bytes_synced: number;
        error: string | null;
      }>;
    }>;
  };
  performance: {
    db_ping_ms: number;
    cache_hit_ratio: number | null;
    cache_size_kib: number;
    wal_checkpoint: {
      busy: number;
      log_pages: number;
      checkpointed_pages: number;
    } | null;
    compile_options: string[];
    read_pool_size: number;
    write_pool_size: number;
    read_pool_idle: number;
    write_pool_idle: number;
  };
  timings: {
    total_ms: number;
    server_ms: number;
    database_ms: number;
    storage_ms: number;
    users_ms: number;
    photos_ms: number;
    audit_ms: number;
    client_logs_ms: number;
    backup_ms: number;
    performance_ms: number;
  };
}

export interface AuditLogEntry {
  id: string;
  event_type: string;
  user_id: string | null;
  username: string | null;
  ip_address: string;
  user_agent: string;
  details: string;
  created_at: string;
  source_server: string | null;
}

export interface AuditLogListResponse {
  logs: AuditLogEntry[];
  next_cursor: string | null;
  total: number;
}

export interface AuditLogParams {
  event_type?: string;
  user_id?: string;
  ip_address?: string;
  after?: string;
  before?: string;
  limit?: number;
  source_server?: string;
}

export interface ClientLogEntry {
  id: string;
  user_id: string;
  session_id: string;
  level: string;
  tag: string;
  message: string;
  context: unknown;
  client_ts: string;
  created_at: string;
}

export interface ClientLogListResponse {
  logs: ClientLogEntry[];
  next_cursor: string | null;
}

export interface ClientLogParams {
  user_id?: string;
  session_id?: string;
  level?: string;
  after?: string;
  limit?: number;
}

/** Union type for the diagnostics endpoint response */
export type DiagnosticsResponseUnion = DiagnosticsResponse | DisabledDiagnosticsResponse;
