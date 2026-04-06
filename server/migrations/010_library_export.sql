-- Library export: tracks zip archive export jobs and their downloadable files.

CREATE TABLE IF NOT EXISTS export_jobs (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',   -- pending, running, completed, failed
    size_limit   INTEGER NOT NULL,                  -- max bytes per zip file
    created_at   TEXT NOT NULL,
    completed_at TEXT,
    error        TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE TABLE IF NOT EXISTS export_files (
    id         TEXT PRIMARY KEY,
    job_id     TEXT NOT NULL,
    filename   TEXT NOT NULL,       -- e.g. "export_part_001.zip"
    file_path  TEXT NOT NULL,       -- path on disk relative to storage root
    size_bytes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,       -- auto-delete after 24 hours
    FOREIGN KEY (job_id) REFERENCES export_jobs(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_export_jobs_user ON export_jobs(user_id);
CREATE INDEX IF NOT EXISTS idx_export_files_job ON export_files(job_id);
CREATE INDEX IF NOT EXISTS idx_export_files_expires ON export_files(expires_at);
