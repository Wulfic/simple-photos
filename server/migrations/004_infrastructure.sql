-- Infrastructure: server settings, backup, client logs.
--
-- Consolidated from: 004_encryption_modes (server_settings), 005_trash_and_backup
-- (backup tables), 006_backup_mode_and_autoscan, 008_client_logs,
-- 018_audio_backup_setting, 019_diagnostics_config, 021_encryption_key_persistence.

-- ── Server Settings ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS server_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Default settings (encryption mode is implicit — always encrypted)
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('trash_retention_days', '30');
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('backup_mode', 'primary');
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('last_auto_scan', '');
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('audio_backup_enabled', 'false');
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('diagnostics_enabled', 'true');
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('client_diagnostics_enabled', 'true');
-- encryption_key_wrapped and encryption_key_active are stored at runtime
-- via the store-key endpoint; no default INSERT needed.

-- ── Backup Servers & Sync Log ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS backup_servers (
    id                   TEXT PRIMARY KEY,
    name                 TEXT NOT NULL,
    address              TEXT NOT NULL UNIQUE,
    api_key              TEXT,
    sync_frequency_hours INTEGER NOT NULL DEFAULT 24,
    last_sync_at         TEXT,
    last_sync_status     TEXT NOT NULL DEFAULT 'never',
    last_sync_error      TEXT,
    enabled              INTEGER NOT NULL DEFAULT 1,
    created_at           TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backup_sync_log (
    id            TEXT PRIMARY KEY,
    server_id     TEXT NOT NULL REFERENCES backup_servers(id) ON DELETE CASCADE,
    started_at    TEXT NOT NULL,
    completed_at  TEXT,
    status        TEXT NOT NULL DEFAULT 'running',
    photos_synced INTEGER NOT NULL DEFAULT 0,
    bytes_synced  INTEGER NOT NULL DEFAULT 0,
    error         TEXT,
    details       TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_sync_log_server ON backup_sync_log(server_id, started_at);

-- ── Client Logs ─────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS client_logs (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    level      TEXT NOT NULL,
    tag        TEXT NOT NULL,
    message    TEXT NOT NULL,
    context    TEXT,
    client_ts  TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_client_logs_user    ON client_logs(user_id);
CREATE INDEX IF NOT EXISTS idx_client_logs_session ON client_logs(session_id);
CREATE INDEX IF NOT EXISTS idx_client_logs_level   ON client_logs(level);
CREATE INDEX IF NOT EXISTS idx_client_logs_created ON client_logs(created_at);
