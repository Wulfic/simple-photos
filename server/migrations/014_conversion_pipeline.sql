-- Track the original file path for converted media files.
-- When a non-native format (e.g. HEIC, MKV) is converted to a browser-native
-- format during import, source_path records the original file location so
-- subsequent scans don't re-convert it.
ALTER TABLE photos ADD COLUMN source_path TEXT;
