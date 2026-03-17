-- Migration 023: Clean up duplicate photo entries from concurrent scan races.
--
-- Prior to the scan mutex (added in this release), concurrent scans could
-- create duplicate rows for the same file when hash computation failed
-- (returning NULL) because the partial UNIQUE index on (user_id, photo_hash)
-- only covers non-NULL hashes.
--
-- We cannot add UNIQUE(user_id, file_path) because the "Save Copy" / duplicate
-- feature intentionally creates multiple rows sharing the same file_path with
-- independent crop/edit metadata.
--
-- The scan mutex now prevents concurrent scans, so this migration only needs
-- to clean up any existing duplicates from before the fix.
--
-- Strategy: for each (user_id, file_path) group with multiple rows where ALL
-- rows look like scan-created entries (not user-created copies), keep only the
-- one with a photo_hash (preferred) or the earliest created_at.
-- Copies are identified by filename starting with "Copy of " — those are
-- preserved regardless.

-- Remove scan-created duplicates only (not user copies).
-- Keep the "best" row: one with photo_hash, or earliest created_at.
DELETE FROM photos
WHERE id NOT IN (
    SELECT id FROM (
        SELECT id,
               ROW_NUMBER() OVER (
                   PARTITION BY user_id, file_path
                   ORDER BY
                       CASE WHEN photo_hash IS NOT NULL THEN 0 ELSE 1 END,
                       created_at ASC
               ) AS rn,
               -- Count how many rows in this group look like copies
               SUM(CASE WHEN filename LIKE 'Copy of %' THEN 1 ELSE 0 END) OVER (
                   PARTITION BY user_id, file_path
               ) AS copy_count,
               -- Whether this specific row is a copy
               CASE WHEN filename LIKE 'Copy of %' THEN 1 ELSE 0 END AS is_copy
        FROM photos
    ) ranked
    -- Keep ALL rows if any copy exists (user intentionally duplicated)
    -- Otherwise keep only rn=1 (the best scan row)
    WHERE copy_count > 0 OR rn = 1
);
