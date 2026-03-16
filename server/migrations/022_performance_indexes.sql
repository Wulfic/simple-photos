-- ──────────────────────────────────────────────────────────────────────────────
-- Migration 022: Performance indexes
--
-- Adds indexes to cover the most frequently-hit query paths that were
-- previously doing full table scans or file-sorts:
--
-- 1. list_photos / encrypted_sync: ORDER BY COALESCE(taken_at, created_at)
-- 2. Blob upload quota check: SUM(size_bytes) WHERE user_id = ?
-- 3. Hourly token cleanup: DELETE WHERE expires_at < ? OR revoked = 1
-- 4. encrypted_sync filter: WHERE encrypted_blob_id IS NOT NULL
-- ──────────────────────────────────────────────────────────────────────────────

-- Photos: cover the list_photos and encrypted_sync sort order.
-- Both endpoints ORDER BY COALESCE(taken_at, created_at) DESC, filename ASC
-- with a WHERE user_id = ? filter.  Without this index SQLite must scan the
-- entire photos table and sort in a temp B-tree on every gallery page load.
CREATE INDEX IF NOT EXISTS idx_photos_user_taken_or_created
    ON photos(user_id, COALESCE(taken_at, created_at) DESC, filename ASC);

-- Photos: cover encrypted_sync which additionally filters on encrypted_blob_id
-- IS NOT NULL.  Partial index so only encrypted photos contribute to the
-- index size — plain-mode rows are excluded.
CREATE INDEX IF NOT EXISTS idx_photos_encrypted_sync
    ON photos(user_id, COALESCE(taken_at, created_at) DESC, filename ASC)
    WHERE encrypted_blob_id IS NOT NULL;

-- Blobs: cover the per-user quota check (SUM(size_bytes) WHERE user_id = ?).
-- This query runs on every blob upload.  The composite index lets SQLite
-- satisfy the SUM by scanning just the user's index entries rather than
-- the entire blobs table.
CREATE INDEX IF NOT EXISTS idx_blobs_user_size
    ON blobs(user_id, size_bytes);

-- Refresh tokens: cover the hourly cleanup query that DELETEs expired and
-- revoked tokens.  The query is:
--   DELETE FROM refresh_tokens WHERE expires_at < ? OR (revoked = 1 AND created_at < ?)
-- This index lets SQLite efficiently find expired tokens without a full scan.
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires
    ON refresh_tokens(expires_at);

-- Photos: cover the is_favorite filter used when favorites_only=true.
-- Without this, SQLite scans all photos for the user to find favorited ones.
CREATE INDEX IF NOT EXISTS idx_photos_user_favorite
    ON photos(user_id, is_favorite)
    WHERE is_favorite = 1;
