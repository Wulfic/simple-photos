-- Add source_server column to audit_log so we can distinguish local events
-- from events forwarded by backup servers.  NULL = local (this server).
ALTER TABLE audit_log ADD COLUMN source_server TEXT;

-- Track the last forwarded audit log timestamp per backup server so the
-- backup only sends new entries on each push cycle.
CREATE TABLE IF NOT EXISTS audit_forward_cursor (
    server_id  TEXT PRIMARY KEY,
    last_id    TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
