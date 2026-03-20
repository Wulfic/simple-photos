-- Core identity & authentication tables.
--
-- Consolidated from: 001_initial, 002_audit_and_lockout, 003_roles.

-- ── Users ───────────────────────────────────────────────────────────────────
CREATE TABLE users (
    id                  TEXT PRIMARY KEY,
    username            TEXT NOT NULL UNIQUE,
    password_hash       TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    storage_quota_bytes INTEGER NOT NULL DEFAULT 10737418240,
    totp_secret         TEXT,
    totp_enabled        INTEGER NOT NULL DEFAULT 0,
    role                TEXT NOT NULL DEFAULT 'user'
);

CREATE TABLE totp_backup_codes (
    id        TEXT PRIMARY KEY,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    used      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_totp_backup_user ON totp_backup_codes(user_id);

CREATE TABLE refresh_tokens (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    revoked    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_refresh_tokens_user    ON refresh_tokens(user_id);
CREATE INDEX idx_refresh_tokens_expires ON refresh_tokens(expires_at);

-- ── Audit & Lockout ─────────────────────────────────────────────────────────
CREATE TABLE audit_log (
    id         TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    user_id    TEXT,
    ip_address TEXT NOT NULL DEFAULT 'unknown',
    user_agent TEXT NOT NULL DEFAULT 'unknown',
    details    TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
CREATE INDEX idx_audit_log_user  ON audit_log(user_id, created_at);
CREATE INDEX idx_audit_log_event ON audit_log(event_type, created_at);
CREATE INDEX idx_audit_log_ip    ON audit_log(ip_address, created_at);

CREATE TABLE account_lockouts (
    user_id         TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    lockout_until   TEXT,
    last_attempt_at TEXT NOT NULL
);
