-- Add is_favorite flag and crop_metadata to photos table
ALTER TABLE photos ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE photos ADD COLUMN crop_metadata TEXT;
