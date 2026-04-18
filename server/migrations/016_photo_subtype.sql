-- Add photo subtype detection columns for motion photos, panorama, 360,
-- HDR, and burst photos.

-- Subtype label: 'motion', 'panorama', 'equirectangular', 'hdr', 'burst'
ALTER TABLE photos ADD COLUMN photo_subtype TEXT;

-- Burst group identifier (shared across all shots in a burst sequence)
ALTER TABLE photos ADD COLUMN burst_id TEXT;

-- Motion photo: blob ID for the extracted embedded MP4 video
ALTER TABLE photos ADD COLUMN motion_video_blob_id TEXT REFERENCES blobs(id);

-- Index for subtype filtering (e.g. GET /api/photos?subtype=motion)
CREATE INDEX IF NOT EXISTS idx_photos_subtype
    ON photos(user_id, photo_subtype)
    WHERE photo_subtype IS NOT NULL;

-- Index for burst grouping queries
CREATE INDEX IF NOT EXISTS idx_photos_burst
    ON photos(user_id, burst_id)
    WHERE burst_id IS NOT NULL;
