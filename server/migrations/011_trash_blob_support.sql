-- Add encrypted blob columns to trash_items so blob-based (encrypted) photos
-- can be soft-deleted into trash and later restored to the blobs table.
ALTER TABLE trash_items ADD COLUMN encrypted_blob_id TEXT;
ALTER TABLE trash_items ADD COLUMN thumbnail_blob_id TEXT;
