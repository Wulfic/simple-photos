-- Track the original blob ID when a photo is cloned into a secure gallery.
-- This allows list_secure_blob_ids to return both original and cloned IDs,
-- ensuring originals are properly hidden from the main gallery view.

ALTER TABLE encrypted_gallery_items ADD COLUMN original_blob_id TEXT;
CREATE INDEX IF NOT EXISTS idx_enc_gallery_items_original ON encrypted_gallery_items(original_blob_id);
