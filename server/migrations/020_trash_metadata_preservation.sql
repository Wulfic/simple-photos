-- Migration 020: Preserve photo metadata through trash/restore cycle
--
-- Previously, soft-deleting a photo to trash lost several metadata fields
-- (is_favorite, crop_metadata, camera_model, photo_hash, encrypted_thumb_blob_id,
-- client_hash, content_hash). Restoring from trash would reset these to defaults,
-- causing data loss.
--
-- This migration adds the missing columns to trash_items so the full round-trip
-- preserves all photo metadata.

-- ── Photo metadata columns ──────────────────────────────────────────────────
ALTER TABLE trash_items ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE trash_items ADD COLUMN crop_metadata TEXT;
ALTER TABLE trash_items ADD COLUMN camera_model TEXT;
ALTER TABLE trash_items ADD COLUMN photo_hash TEXT;
ALTER TABLE trash_items ADD COLUMN encrypted_thumb_blob_id TEXT;

-- ── Blob hash columns (for dedup/integrity after restore) ───────────────────
ALTER TABLE trash_items ADD COLUMN client_hash TEXT;
ALTER TABLE trash_items ADD COLUMN content_hash TEXT;

-- ── Performance: index on encrypted_blob_id for thumbnail lookups ───────────
-- The encrypted-mode thumbnail endpoint queries:
--   SELECT encrypted_thumb_blob_id FROM photos WHERE encrypted_blob_id = ? AND user_id = ?
-- Without this index, every thumbnail request does a full table scan.
CREATE INDEX IF NOT EXISTS idx_photos_encrypted_blob
    ON photos(encrypted_blob_id)
    WHERE encrypted_blob_id IS NOT NULL;
