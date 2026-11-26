-- Migration 043: an undo log for photo date repairs (TODO.md §4b P1b)
--
-- 5,003 of 5,146 Android-uploaded rows on the live box were stored with
-- taken_at = 1970-01-01: the phone read only MediaStore DATE_TAKEN, which is
-- NULL for a downloaded file. Only the phone knows the right date (the blob is
-- encrypted before upload; the server's created_at is the upload time), so the
-- phone sends corrections through `POST /api/photos/dates/repair`.
--
-- That rewrites thousands of rows the user can see. Every applied change writes
-- its old values here first, under one batch id, so
-- `POST /api/photos/dates/repair/undo {batch_id}` can put each row back exactly
-- (taken_at, photo_year, photo_month and taken_at_offset — the repair clears
-- the offset, so it must be restorable too).
--
-- No FK to photos(id): an undo record must not vanish with a trashed photo
-- (a restore re-inserts the same id). Undo of a row that no longer exists is
-- reported as `not_found` and changes nothing.
--
-- Reverse (forward-only sqlx migrations have no down file):
--   DROP INDEX IF EXISTS idx_taken_at_repairs_user_batch;
--   DROP TABLE IF EXISTS taken_at_repairs;
-- Loses only the undo history; repaired dates stay as repaired. The binary may
-- be rolled back without it (`set_ignore_missing`; old code never reads it).
-- `date_repair::tests::the_documented_reverse_drops_the_table` runs it.

CREATE TABLE IF NOT EXISTS taken_at_repairs (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id            TEXT NOT NULL,
    user_id             TEXT NOT NULL,
    photo_id            TEXT NOT NULL,
    old_taken_at        TEXT,
    old_photo_year      INTEGER,
    old_photo_month     INTEGER,
    old_taken_at_offset TEXT,
    new_taken_at        TEXT NOT NULL,
    repaired_at         TEXT NOT NULL,
    -- NULL until undone; an undone batch cannot be undone twice.
    undone_at           TEXT
);

CREATE INDEX IF NOT EXISTS idx_taken_at_repairs_user_batch
    ON taken_at_repairs (user_id, batch_id);
