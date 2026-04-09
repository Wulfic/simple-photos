-- Track the encrypted thumbnail blob ID on gallery items so
-- clients (and backup servers) can locate thumbnails without
-- needing a JOIN to the photos table.
ALTER TABLE encrypted_gallery_items ADD COLUMN encrypted_thumb_blob_id TEXT;
