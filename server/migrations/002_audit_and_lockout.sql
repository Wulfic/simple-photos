-- Security audit log: immutable record of all security-relevant events.
-- Used for incident response, compliance, and debugging.
CREATE TABLE IF NOT EXISTS audit_log (
    id          TEXT PRIMARY KEY,
    event_type  TEXT NOT NULL,
    user_id     TEXT,
    ip_address  TEXT NOT NULL DEFAULT 'unknown',
    user_agent  TEXT NOT NULL DEFAULT 'unknown',
    details     TEXT NOT NULL DEFAULT '{}',
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_log_user ON audit_log(user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_audit_log_event ON audit_log(event_type, created_at);
CREATE INDEX IF NOT EXISTS idx_audit_log_ip ON audit_log(ip_address, created_at);

-- Account lockout tracking: lock accounts after too many failed login attempts.
-- Unlocks after lockout_until has passed.
CREATE TABLE IF NOT EXISTS account_lockouts (
    user_id        TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    lockout_until   TEXT,
    last_attempt_at TEXT NOT NULL
);
