-- Track the original photo's content hash in encrypted_gallery_items so that
-- autoscan (run after recovery) can skip files whose content matches a
-- gallery-hidden original.  Content hashing is rename/move proof — works even
-- if the file gets a different name or is relocated on disk.
--
-- Uses the same 48-bit SHA-256 prefix (12 hex chars) as photos.photo_hash
-- and blobs.content_hash.
ALTER TABLE encrypted_gallery_items ADD COLUMN original_photo_hash TEXT;

-- Back-fill from existing photos rows (only works while the original photo
-- row still exists, i.e. before the primary is wiped for recovery).
UPDATE encrypted_gallery_items
   SET original_photo_hash = (
       SELECT photo_hash FROM photos WHERE id = encrypted_gallery_items.original_blob_id
   )
 WHERE original_blob_id IS NOT NULL
   AND original_photo_hash IS NULL;
