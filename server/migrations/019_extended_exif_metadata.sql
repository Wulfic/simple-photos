-- Extended EXIF metadata columns on the photos table.
-- These store commonly-edited EXIF fields separately for fast queries and
-- inline editing. Values are synced from file EXIF on upload and can be
-- written back via the metadata write-exif endpoint.

ALTER TABLE photos ADD COLUMN camera_make        TEXT;
ALTER TABLE photos ADD COLUMN lens_model         TEXT;
ALTER TABLE photos ADD COLUMN iso_speed          INTEGER;
ALTER TABLE photos ADD COLUMN f_number           REAL;
ALTER TABLE photos ADD COLUMN exposure_time      TEXT;
ALTER TABLE photos ADD COLUMN focal_length       REAL;
ALTER TABLE photos ADD COLUMN flash              TEXT;
ALTER TABLE photos ADD COLUMN white_balance      TEXT;
ALTER TABLE photos ADD COLUMN exposure_program   TEXT;
ALTER TABLE photos ADD COLUMN metering_mode      TEXT;
ALTER TABLE photos ADD COLUMN orientation        INTEGER;
ALTER TABLE photos ADD COLUMN software           TEXT;
ALTER TABLE photos ADD COLUMN artist             TEXT;
ALTER TABLE photos ADD COLUMN copyright          TEXT;
ALTER TABLE photos ADD COLUMN description        TEXT;
ALTER TABLE photos ADD COLUMN user_comment       TEXT;
ALTER TABLE photos ADD COLUMN color_space        TEXT;
ALTER TABLE photos ADD COLUMN exposure_bias      REAL;
ALTER TABLE photos ADD COLUMN scene_type         TEXT;
ALTER TABLE photos ADD COLUMN digital_zoom       REAL;

-- Arbitrary EXIF overrides (JSON object of tag → value).
-- Used for tags that don't have dedicated columns.
ALTER TABLE photos ADD COLUMN exif_overrides     TEXT;
