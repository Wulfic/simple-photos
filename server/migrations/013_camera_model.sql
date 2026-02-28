-- Add camera_model column to photos table for storing EXIF device info
ALTER TABLE photos ADD COLUMN camera_model TEXT;
