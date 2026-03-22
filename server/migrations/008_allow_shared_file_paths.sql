-- Migration 008: allow photo copies to share a file_path with their original.
--
-- The old partial unique index on (user_id, file_path) WHERE file_path != ''
-- prevented the duplicate_photo endpoint from creating a copy row when the
-- original's file_path is non-empty, because two rows would share the same
-- (user_id, file_path) pair.  Copies intentionally re-use the original's
-- file on disk (no data is duplicated), so the constraint must only apply
-- to "canonical" photo rows that own their file, not to copies.
--
-- Canonical rows have photo_hash set (computed from the file content).
-- Copy rows always have photo_hash = NULL (enforced by duplicate_photo).
-- Restricting the uniqueness check to rows WHERE photo_hash IS NOT NULL
-- allows multiple copy rows to reference the same file while still
-- preventing accidental duplicate imports of the same physical file.

DROP INDEX IF EXISTS idx_photos_unique_file_path;

CREATE UNIQUE INDEX IF NOT EXISTS idx_photos_unique_file_path
    ON photos(user_id, file_path)
    WHERE file_path != '' AND photo_hash IS NOT NULL;
