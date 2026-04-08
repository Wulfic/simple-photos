-- Track the encrypted_blob_id for secure gallery items.
--
-- On the primary, list_gallery_items resolves this via a JOIN on photos.
-- On the backup, the photos row may not exist (clones excluded from sync),
-- so we store the encrypted_blob_id directly in the gallery items table
-- (populated by the gallery metadata sync from the primary).
ALTER TABLE encrypted_gallery_items ADD COLUMN encrypted_blob_id TEXT;
