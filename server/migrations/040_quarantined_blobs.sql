-- Migration 040: quarantine for orphan blobs (TODO.md F1)
--
-- A phone that re-uploads content the server already holds leaves one stored
-- thumbnail blob per photo that nothing references (CT132, 2026-11-25: 18,067
-- of them, 754 MB). `maintenance::orphan_thumbs` reclaims them, but never by
-- unlinking: it moves the `blobs` row here and the file under
-- `<storage_root>/.quarantine/<run_id>/<storage_path>`. Restore moves both back;
-- purge is a separate, explicit call and the only thing that unlinks.
--
-- Columns mirror `blobs` (002 + 024's `blob_format`) so a restore is a plain
-- INSERT ... SELECT with nothing to reconstruct. The orphan_thumbs test
-- `quarantine_table_carries_every_blobs_column` fails if `blobs` gains a column
-- this table does not have.
--
-- No FK to users or blobs: the point of this table is to outlive the `blobs`
-- row, and a deleted user's quarantine is still purgeable by run_id.
--
-- Reverse (forward-only sqlx migrations have no down file):
--   DROP TABLE IF EXISTS quarantined_blobs;
-- Only safe once the table is empty — every row is a blob whose file sits in
-- the quarantine dir and whose `blobs` row exists nowhere else.

CREATE TABLE IF NOT EXISTS quarantined_blobs (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL,
    user_id         TEXT NOT NULL,
    blob_type       TEXT NOT NULL,
    size_bytes      INTEGER NOT NULL,
    client_hash     TEXT,
    upload_time     TEXT NOT NULL,
    storage_path    TEXT NOT NULL,
    content_hash    TEXT,
    blob_format     INTEGER NOT NULL DEFAULT 1,
    quarantine_path TEXT NOT NULL,
    quarantined_at  TEXT NOT NULL,
    reason          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_quarantined_blobs_run ON quarantined_blobs(run_id);
