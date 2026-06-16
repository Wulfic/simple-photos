-- Track the deleted photo's original on-disk plaintext path in trash.
--
-- Root cause of "trashed photos reappear after a minute": server-side
-- encryption keeps the plaintext original on disk (photos.file_path stays
-- set, the file is never removed). The encrypted-blob soft-delete path stored
-- only the *blob* storage_path in trash_items.file_path and then deleted the
-- photos row, so the filesystem autoscan's exclusion set no longer matched the
-- orphaned plaintext original and re-imported it on the next scan.
--
-- We now capture that original path so the autoscan can skip it while the item
-- is in trash, and so the purge / permanent-delete / empty-trash paths can
-- remove the plaintext original once the trash row is gone (otherwise dropping
-- the row would re-expose the file to the next scan).
ALTER TABLE trash_items ADD COLUMN original_file_path TEXT;

-- Lets the autoscan exclusion query filter on this column cheaply.
CREATE INDEX IF NOT EXISTS idx_trash_original_path
    ON trash_items(original_file_path);
