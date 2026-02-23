-- Trash: soft-deleted photos are moved here with a 30-day retention period.
-- After 30 days, a background task permanently deletes them.
CREATE TABLE IF NOT EXISTS trash_items (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    photo_id      TEXT NOT NULL,            -- original photo ID (removed from photos table)
    filename      TEXT NOT NULL,
    file_path     TEXT NOT NULL,            -- relative path within storage root
    mime_type     TEXT NOT NULL,
    media_type    TEXT NOT NULL DEFAULT 'photo',
    size_bytes    INTEGER NOT NULL DEFAULT 0,
    width         INTEGER NOT NULL DEFAULT 0,
    height        INTEGER NOT NULL DEFAULT 0,
    duration_secs REAL,
    taken_at      TEXT,
    latitude      REAL,
    longitude     REAL,
    thumb_path    TEXT,
    deleted_at    TEXT NOT NULL,            -- ISO 8601 — when the user deleted it
    expires_at    TEXT NOT NULL             -- ISO 8601 — when it will be permanently deleted (deleted_at + 30 days)
);
CREATE INDEX IF NOT EXISTS idx_trash_user ON trash_items(user_id, deleted_at);
CREATE INDEX IF NOT EXISTS idx_trash_expires ON trash_items(expires_at);

-- Backup servers: secondary Simple Photos instances that mirror all data.
CREATE TABLE IF NOT EXISTS backup_servers (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,          -- friendly display name
    address             TEXT NOT NULL UNIQUE,   -- IP or DNS (e.g. "192.168.1.50:8080" or "backup.local:8080")
    api_key             TEXT,                   -- shared secret for authenticating sync requests
    sync_frequency_hours INTEGER NOT NULL DEFAULT 24,
    last_sync_at        TEXT,                   -- ISO 8601 — last successful sync
    last_sync_status    TEXT NOT NULL DEFAULT 'never', -- 'never', 'success', 'error'
    last_sync_error     TEXT,                   -- error message if last sync failed
    enabled             INTEGER NOT NULL DEFAULT 1,
    created_at          TEXT NOT NULL
);

-- Sync log: track individual sync operations for debugging and audit.
CREATE TABLE IF NOT EXISTS backup_sync_log (
    id              TEXT PRIMARY KEY,
    server_id       TEXT NOT NULL REFERENCES backup_servers(id) ON DELETE CASCADE,
    started_at      TEXT NOT NULL,
    completed_at    TEXT,
    status          TEXT NOT NULL DEFAULT 'running', -- 'running', 'success', 'error'
    photos_synced   INTEGER NOT NULL DEFAULT 0,
    bytes_synced    INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    details         TEXT NOT NULL DEFAULT '{}'       -- JSON with additional info
);
CREATE INDEX IF NOT EXISTS idx_sync_log_server ON backup_sync_log(server_id, started_at);

-- Add trash_retention_days to server_settings (default 30).
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('trash_retention_days', '30');
