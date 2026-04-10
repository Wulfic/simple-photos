-- Normalize EXIF-format timestamps and other non-standard date formats in
-- taken_at / created_at to canonical ISO-8601 (YYYY-MM-DDTHH:MM:SSZ).
--
-- The Rust normalize_iso_timestamp() function handles these conversions for
-- new data; this migration fixes any existing rows that were stored before
-- the enhanced normalization was added.
--
-- Safe to re-run: each UPDATE includes a WHERE clause that only matches
-- rows still in the non-standard format.

-- Convert EXIF-format taken_at: "2024:01:15 14:30:00" → "2024-01-15T14:30:00Z"
UPDATE photos
SET taken_at = SUBSTR(taken_at, 1, 4) || '-' || SUBSTR(taken_at, 6, 2) || '-' || SUBSTR(taken_at, 9, 2) || 'T' || SUBSTR(taken_at, 12, 8) || 'Z'
WHERE taken_at IS NOT NULL
  AND LENGTH(taken_at) >= 19
  AND SUBSTR(taken_at, 5, 1) = ':'
  AND SUBSTR(taken_at, 8, 1) = ':';

-- Convert space-separated datetime: "2024-01-15 14:30:00" → "2024-01-15T14:30:00Z"
UPDATE photos
SET taken_at = SUBSTR(taken_at, 1, 10) || 'T' || SUBSTR(taken_at, 12) || 'Z'
WHERE taken_at IS NOT NULL
  AND LENGTH(taken_at) >= 19
  AND SUBSTR(taken_at, 11, 1) = ' '
  AND SUBSTR(taken_at, 5, 1) = '-'
  AND taken_at NOT LIKE '%Z'
  AND taken_at NOT LIKE '%T%';

-- Convert space-separated created_at: "2024-01-15 14:30:00" → "2024-01-15T14:30:00Z"
UPDATE photos
SET created_at = SUBSTR(created_at, 1, 10) || 'T' || SUBSTR(created_at, 12) || 'Z'
WHERE created_at IS NOT NULL
  AND LENGTH(created_at) >= 19
  AND SUBSTR(created_at, 11, 1) = ' '
  AND SUBSTR(created_at, 5, 1) = '-'
  AND created_at NOT LIKE '%Z'
  AND created_at NOT LIKE '%T%';

-- Same conversions for trash_items
UPDATE trash_items
SET taken_at = SUBSTR(taken_at, 1, 4) || '-' || SUBSTR(taken_at, 6, 2) || '-' || SUBSTR(taken_at, 9, 2) || 'T' || SUBSTR(taken_at, 12, 8) || 'Z'
WHERE taken_at IS NOT NULL
  AND LENGTH(taken_at) >= 19
  AND SUBSTR(taken_at, 5, 1) = ':'
  AND SUBSTR(taken_at, 8, 1) = ':';

UPDATE trash_items
SET taken_at = SUBSTR(taken_at, 1, 10) || 'T' || SUBSTR(taken_at, 12) || 'Z'
WHERE taken_at IS NOT NULL
  AND LENGTH(taken_at) >= 19
  AND SUBSTR(taken_at, 11, 1) = ' '
  AND SUBSTR(taken_at, 5, 1) = '-'
  AND taken_at NOT LIKE '%Z'
  AND taken_at NOT LIKE '%T%';

-- Ensure all taken_at values end with millisecond precision and Z
-- "2024-01-15T14:30:00Z" → "2024-01-15T14:30:00.000Z"
UPDATE photos
SET taken_at = SUBSTR(taken_at, 1, LENGTH(taken_at) - 1) || '.000Z'
WHERE taken_at IS NOT NULL
  AND taken_at LIKE '%Z'
  AND taken_at NOT LIKE '%.___Z'
  AND LENGTH(taken_at) = 20;

UPDATE photos
SET created_at = SUBSTR(created_at, 1, LENGTH(created_at) - 1) || '.000Z'
WHERE created_at IS NOT NULL
  AND created_at LIKE '%Z'
  AND created_at NOT LIKE '%.___Z'
  AND LENGTH(created_at) = 20;
