-- Remove all vestiges of plain (unencrypted) mode.
--
-- Prerequisites:
--   - Migration 024 already set encryption_mode = 'encrypted'.
--   - server_migrate.rs should have encrypted all remaining plain photos.
--
-- Safety: this migration will fail if any photos still have NULL encrypted_blob_id.
-- If that happens, run the server with the old code first to finish migration.

-- ── 1. Verify no un-migrated photos remain ─────────────────────────────────
-- SQLite doesn't have procedural DO blocks, so we use a trick:
-- INSERT into a temp table that will fail the CHECK constraint if any
-- un-migrated photos exist.
CREATE TEMPORARY TABLE _migration_check (ok INTEGER CHECK(ok = 0));
INSERT INTO _migration_check (ok)
  SELECT COUNT(*) FROM photos WHERE encrypted_blob_id IS NULL;
DROP TABLE _migration_check;

-- ── 2. Remove the encryption_mode setting row ──────────────────────────────
DELETE FROM server_settings WHERE key = 'encryption_mode';

-- ── 3. Drop the is_encrypted column from photo_metadata ────────────────────
-- SQLite doesn't support DROP COLUMN before 3.35.0, so we recreate the table.
CREATE TABLE photo_metadata_new (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL,
    photo_id      TEXT,
    blob_id       TEXT,
    source        TEXT NOT NULL,
    title         TEXT,
    description   TEXT,
    taken_at      TEXT,
    created_at_src TEXT,
    latitude      REAL,
    longitude     REAL,
    altitude      REAL,
    image_views   INTEGER,
    original_url  TEXT,
    storage_path  TEXT,
    imported_at   TEXT NOT NULL
);

INSERT INTO photo_metadata_new
  SELECT id, user_id, photo_id, blob_id, source, title, description, taken_at,
         created_at_src, latitude, longitude, altitude, image_views, original_url,
         storage_path, imported_at
  FROM photo_metadata;

DROP TABLE photo_metadata;
ALTER TABLE photo_metadata_new RENAME TO photo_metadata;

-- Re-create indexes that existed on photo_metadata (if any)
CREATE INDEX IF NOT EXISTS idx_photo_metadata_user ON photo_metadata(user_id);
CREATE INDEX IF NOT EXISTS idx_photo_metadata_photo ON photo_metadata(photo_id);
CREATE INDEX IF NOT EXISTS idx_photo_metadata_blob ON photo_metadata(blob_id);

-- ── 4. Make encrypted_blob_id NOT NULL on photos ───────────────────────────
-- SQLite requires table recreation: copy data, drop old, rename new.
-- We also drop file_path and thumb_path (plain-mode only columns).
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
    encrypted_blob_id       TEXT NOT NULL,
    encrypted_thumb_blob_id TEXT,
    is_favorite             INTEGER NOT NULL DEFAULT 0,
    crop_metadata           TEXT,
    camera_model            TEXT,
    photo_hash              TEXT
);

INSERT INTO photos_new
  SELECT id, user_id, filename, COALESCE(file_path, ''), mime_type, media_type,
         size_bytes, width, height, duration_secs, taken_at, latitude, longitude,
         thumb_path, created_at, encrypted_blob_id, encrypted_thumb_blob_id,
         is_favorite, crop_metadata, camera_model, photo_hash
  FROM photos;

DROP TABLE photos;
ALTER TABLE photos_new RENAME TO photos;

-- Re-create all indexes from previous migrations
CREATE INDEX IF NOT EXISTS idx_photos_user ON photos(user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_photos_file_path ON photos(file_path);
CREATE INDEX IF NOT EXISTS idx_photos_taken_at ON photos(user_id, taken_at);
CREATE INDEX IF NOT EXISTS idx_photos_encrypted_blob ON photos(encrypted_blob_id);
CREATE INDEX IF NOT EXISTS idx_photos_user_favorite ON photos(user_id, is_favorite);
CREATE INDEX IF NOT EXISTS idx_photos_hash ON photos(photo_hash);
-- Partial index for encrypted sync (all photos are now encrypted)
CREATE INDEX IF NOT EXISTS idx_photos_encrypted_sync
    ON photos(user_id, COALESCE(taken_at, created_at) DESC, filename ASC);
-- Unique constraint on file_path (from migration 023) — only for non-empty paths
CREATE UNIQUE INDEX IF NOT EXISTS idx_photos_unique_file_path
    ON photos(user_id, file_path) WHERE file_path != '';
