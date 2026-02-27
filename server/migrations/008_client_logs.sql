-- Client diagnostic logs: receives structured log entries from mobile
-- clients to help debug backup/upload issues remotely.

CREATE TABLE client_logs (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id  TEXT NOT NULL,       -- groups logs from one backup run
    level       TEXT NOT NULL,       -- 'debug', 'info', 'warn', 'error'
    tag         TEXT NOT NULL,       -- e.g. 'BackupWorker', 'PhotoRepository'
    message     TEXT NOT NULL,
    context     TEXT,                -- optional JSON with extra key-value pairs
    client_ts   TEXT NOT NULL,       -- timestamp on the client device
    created_at  TEXT NOT NULL        -- timestamp when the server received it
);
CREATE INDEX idx_client_logs_user     ON client_logs(user_id);
CREATE INDEX idx_client_logs_session  ON client_logs(session_id);
CREATE INDEX idx_client_logs_level    ON client_logs(level);
CREATE INDEX idx_client_logs_created  ON client_logs(created_at);
