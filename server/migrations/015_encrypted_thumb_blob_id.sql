-- Track the thumbnail blob ID alongside the encrypted photo blob ID.
-- This allows the Android/iOS clients to sync photo metadata from the
-- photos table and then download only the small thumbnail blob for
-- gallery display, instead of downloading the full encrypted photo blob.
ALTER TABLE photos ADD COLUMN encrypted_thumb_blob_id TEXT;
