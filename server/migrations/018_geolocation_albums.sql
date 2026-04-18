-- Resolved location cache (per-photo, from reverse geocoding)
ALTER TABLE photos ADD COLUMN geo_city TEXT;
ALTER TABLE photos ADD COLUMN geo_state TEXT;
ALTER TABLE photos ADD COLUMN geo_country TEXT;
ALTER TABLE photos ADD COLUMN geo_country_code TEXT;
CREATE INDEX idx_photos_geo_country ON photos(user_id, geo_country) WHERE geo_country IS NOT NULL;
CREATE INDEX idx_photos_geo_city ON photos(user_id, geo_city) WHERE geo_city IS NOT NULL;

-- Timestamp-based grouping cache (derived from taken_at or created_at)
ALTER TABLE photos ADD COLUMN photo_year INTEGER;
ALTER TABLE photos ADD COLUMN photo_month INTEGER;
CREATE INDEX idx_photos_year_month ON photos(user_id, photo_year, photo_month) WHERE photo_year IS NOT NULL;
