-- Make encrypted_blob_id nullable on the photos table.
--
-- Server-scanned ("plain") photos have no encrypted_blob_id because they
-- have not been uploaded by a client as an encrypted blob.  The NOT NULL
-- constraint on this column prevented the autoscan from registering such
-- files (INSERT OR IGNORE silently dropped every row).
--
-- SQLite does not support ALTER COLUMN, so we use the 12-step schema change:
--   1. Create replacement table with the corrected schema.
--   2. Copy all existing rows.
--   3. Drop the old table (drops its indexes too).
--   4. Rename replacement → photos (indexes follow the rename).

PRAGMA foreign_keys = OFF;

CREATE TABLE photos_new (
    id                      TEXT PRIMARY KEY,
    user_id                 TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    filename                TEXT NOT NULL,
    file_path               TEXT NOT NULL DEFAULT '',
    mime_type               TEXT NOT NULL,
    media_type              TEXT NOT NULL DEFAULT 'photo',
    size_bytes              INTEGER NOT NULL DEFAULT 0,
    width                   INTEGER NOT NULL DEFAULT 0,
    height                  INTEGER NOT NULL DEFAULT 0,
    duration_secs           REAL,
    taken_at                TEXT,
    latitude                REAL,
    longitude               REAL,
    thumb_path              TEXT,
    created_at              TEXT NOT NULL,
    encrypted_blob_id       TEXT,               -- nullable: NULL for server-scanned plain photos
    encrypted_thumb_blob_id TEXT,
    is_favorite             INTEGER NOT NULL DEFAULT 0,
    crop_metadata           TEXT,
    camera_model            TEXT,
    photo_hash              TEXT
);

INSERT INTO photos_new SELECT * FROM photos;

DROP TABLE photos;
ALTER TABLE photos_new RENAME TO photos;

-- Recreate all indexes (they were dropped with the old table).
CREATE INDEX IF NOT EXISTS idx_photos_user                ON photos(user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_photos_file_path           ON photos(file_path);
CREATE INDEX IF NOT EXISTS idx_photos_taken_at            ON photos(user_id, taken_at);
CREATE INDEX IF NOT EXISTS idx_photos_encrypted_blob      ON photos(encrypted_blob_id) WHERE encrypted_blob_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_photos_user_favorite       ON photos(user_id, is_favorite) WHERE is_favorite = 1;
CREATE INDEX IF NOT EXISTS idx_photos_hash                ON photos(photo_hash);
CREATE UNIQUE INDEX IF NOT EXISTS idx_photos_user_hash    ON photos(user_id, photo_hash) WHERE photo_hash IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_photos_unique_file_path
    ON photos(user_id, file_path) WHERE file_path != '';
CREATE INDEX IF NOT EXISTS idx_photos_encrypted_sync
    ON photos(user_id, COALESCE(taken_at, created_at), filename);
CREATE INDEX IF NOT EXISTS idx_photos_user_taken_or_created
    ON photos(user_id, COALESCE(taken_at, created_at), filename);

PRAGMA foreign_keys = ON;
