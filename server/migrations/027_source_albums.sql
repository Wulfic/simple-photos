-- Authoritative source-album tracking for imported media.
--
-- Google Takeout lays each photo out under its album folder (plus non-album
-- "Photos from YYYY" date folders). The importer used to read that parent-folder
-- album name and throw it away, flattening every file into a single gallery —
-- so "folders are not maintained" after import. Album re-creation was a manual,
-- web-only, filename-matching second step that most users never ran.
--
-- This table captures the album membership AT IMPORT TIME, keyed by the
-- authoritative `photo_id` (not by fragile filename matching), so any client
-- can rebuild album manifests deterministically and cross-platform. A photo can
-- appear in multiple album folders (Takeout duplicates it), hence the composite
-- primary key rather than a single column on `photos`.
CREATE TABLE IF NOT EXISTS photo_source_albums (
    photo_id    TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    album_name  TEXT NOT NULL,
    source      TEXT NOT NULL DEFAULT 'google_takeout',
    created_at  TEXT NOT NULL,
    PRIMARY KEY (photo_id, album_name),
    FOREIGN KEY (photo_id) REFERENCES photos(id) ON DELETE CASCADE
);

-- Fast "all source albums for this user" lookup used by the rebuild endpoint.
CREATE INDEX IF NOT EXISTS idx_photo_source_albums_user
    ON photo_source_albums (user_id, album_name);
