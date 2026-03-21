-- Backup server diagnostics: store the latest health report pushed
-- by each backup server to the primary so admins can see the backup's
-- health without having to query it directly.
--
-- last_diagnostics     — JSON blob containing the most recent report
-- last_diagnostics_at  — RFC-3339 timestamp of when the report was received

ALTER TABLE backup_servers ADD COLUMN last_diagnostics     TEXT;
ALTER TABLE backup_servers ADD COLUMN last_diagnostics_at  TEXT;
