-- Google Photos metadata import support.
-- Stores parsed metadata from Google Photos JSON sidecar files,
-- linked to photos/blobs via photo_id or blob_id.
-- Metadata can be stored as a separate file in the metadata/ subdirectory
-- or packed directly with the blob. If encrypted mode is active,
-- metadata files are encrypted at rest in the metadata/ directory.

CREATE TABLE photo_metadata (
    id              TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    photo_id        TEXT,                           -- links to photos.id (plain mode)
    blob_id         TEXT,                           -- links to blobs.id (encrypted mode)
    source          TEXT NOT NULL DEFAULT 'manual', -- 'google_photos', 'manual', 'exif'
    title           TEXT,
    description     TEXT,
    taken_at        TEXT,                           -- ISO 8601 from photoTakenTime
    created_at_src  TEXT,                           -- creationTime from source
    latitude        REAL,
    longitude       REAL,
    altitude        REAL,
    image_views     INTEGER,
    original_url    TEXT,                           -- Google Photos URL
    storage_path    TEXT,                           -- relative path to metadata JSON in metadata/ dir
    is_encrypted    INTEGER NOT NULL DEFAULT 0,     -- 1 if metadata file is encrypted
    imported_at     TEXT NOT NULL
);

CREATE INDEX idx_photo_metadata_user ON photo_metadata(user_id);
CREATE INDEX idx_photo_metadata_photo ON photo_metadata(photo_id) WHERE photo_id IS NOT NULL;
CREATE INDEX idx_photo_metadata_blob ON photo_metadata(blob_id) WHERE blob_id IS NOT NULL;
CREATE INDEX idx_photo_metadata_source ON photo_metadata(user_id, source);
