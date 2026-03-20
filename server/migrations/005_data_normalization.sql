-- Data normalization: timestamp cleanup.
--
-- Consolidated from: 016_normalize_timestamps.
-- Applied once; safe to re-run (WHERE clauses guard against double-application).

-- Normalize taken_at: ensure trailing 'Z' for UTC
UPDATE photos
SET taken_at = taken_at || 'Z'
WHERE taken_at IS NOT NULL
  AND taken_at NOT LIKE '%Z'
  AND taken_at NOT LIKE '%+00:00';

UPDATE photos
SET taken_at = SUBSTR(taken_at, 1, LENGTH(taken_at) - 6) || 'Z'
WHERE taken_at IS NOT NULL
  AND taken_at LIKE '%+00:00';

-- Normalize created_at: ensure trailing 'Z' for UTC
UPDATE photos
SET created_at = created_at || 'Z'
WHERE created_at IS NOT NULL
  AND created_at NOT LIKE '%Z'
  AND created_at NOT LIKE '%+00:00';

UPDATE photos
SET created_at = SUBSTR(created_at, 1, LENGTH(created_at) - 6) || 'Z'
WHERE created_at IS NOT NULL
  AND created_at LIKE '%+00:00';
