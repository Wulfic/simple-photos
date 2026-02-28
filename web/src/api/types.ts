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

export interface DiagnosticsResponse {
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
    plain_count: number;
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
  };
  performance: {
    db_ping_ms: number;
    cache_hit_ratio: number | null;
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
