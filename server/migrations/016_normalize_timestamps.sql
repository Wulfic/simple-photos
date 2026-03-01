-- Normalize all timestamps in the photos table to consistent ISO-8601 format
-- with Z suffix for correct lexicographic sorting in SQLite.
--
-- Converts:
--   "2024-01-15T14:30:00"              →  "2024-01-15T14:30:00Z"      (naive → UTC)
--   "2024-01-15T14:30:00.000000+00:00" →  "2024-01-15T14:30:00.000000Z" (offset → Z)
--   "2024-01-15T14:30:00.043Z"         →  unchanged                   (already correct)

-- Fix taken_at: naive timestamps (no timezone suffix) — append Z
UPDATE photos
SET taken_at = taken_at || 'Z'
WHERE taken_at IS NOT NULL
  AND taken_at NOT LIKE '%Z'
  AND taken_at NOT LIKE '%+00:00';

-- Fix taken_at: replace +00:00 offset with Z
UPDATE photos
SET taken_at = SUBSTR(taken_at, 1, LENGTH(taken_at) - 6) || 'Z'
WHERE taken_at IS NOT NULL
  AND taken_at LIKE '%+00:00';

-- Fix created_at: naive timestamps — append Z
UPDATE photos
SET created_at = created_at || 'Z'
WHERE created_at IS NOT NULL
  AND created_at NOT LIKE '%Z'
  AND created_at NOT LIKE '%+00:00';

-- Fix created_at: replace +00:00 offset with Z
UPDATE photos
SET created_at = SUBSTR(created_at, 1, LENGTH(created_at) - 6) || 'Z'
WHERE created_at IS NOT NULL
  AND created_at LIKE '%+00:00';
