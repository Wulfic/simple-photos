-- Server-wide settings (key-value store for global configuration).
CREATE TABLE IF NOT EXISTS server_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Default: photos are stored as plain (unencrypted) files.
-- 'plain' = original files on disk, metadata in photos table
-- 'encrypted' = client-side encrypted blobs (current behavior)
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('encryption_mode', 'plain');

-- Plain-mode photo metadata.
-- When encryption_mode = 'plain', photos are stored as original files on disk.
-- This table tracks their metadata so the gallery can display them without decryption.
CREATE TABLE IF NOT EXISTS photos (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    filename      TEXT NOT NULL,
    file_path     TEXT NOT NULL,          -- relative path within storage root
    mime_type     TEXT NOT NULL,
    media_type    TEXT NOT NULL DEFAULT 'photo',  -- 'photo', 'gif', 'video'
    size_bytes    INTEGER NOT NULL DEFAULT 0,
    width         INTEGER NOT NULL DEFAULT 0,
    height        INTEGER NOT NULL DEFAULT 0,
    duration_secs REAL,                   -- video duration (NULL for images)
    taken_at      TEXT,                   -- ISO 8601
    latitude      REAL,
    longitude     REAL,
    thumb_path    TEXT,                   -- relative path to generated thumbnail
    created_at    TEXT NOT NULL,
    -- If this photo has been migrated to encrypted storage, store the blob_id
    encrypted_blob_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_photos_user ON photos(user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_photos_file_path ON photos(file_path);

-- Is encryption migration currently in progress?
-- Tracks background migration between plain ↔ encrypted modes.
CREATE TABLE IF NOT EXISTS encryption_migration (
    id          TEXT PRIMARY KEY DEFAULT 'singleton',
    status      TEXT NOT NULL DEFAULT 'idle',   -- 'idle', 'encrypting', 'decrypting'
    total       INTEGER NOT NULL DEFAULT 0,
    completed   INTEGER NOT NULL DEFAULT 0,
    started_at  TEXT,
    error       TEXT
);
INSERT OR IGNORE INTO encryption_migration (id) VALUES ('singleton');

-- Encrypted galleries — always encrypted, independent of global setting.
-- Each gallery has its own password (bcrypt hash).
CREATE TABLE IF NOT EXISTS encrypted_galleries (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    password_hash TEXT NOT NULL,           -- bcrypt hash of gallery password
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_enc_galleries_user ON encrypted_galleries(user_id);

-- Photos belonging to encrypted galleries.
-- These are always stored as encrypted blobs regardless of global encryption_mode.
CREATE TABLE IF NOT EXISTS encrypted_gallery_items (
    id          TEXT PRIMARY KEY,
    gallery_id  TEXT NOT NULL REFERENCES encrypted_galleries(id) ON DELETE CASCADE,
    blob_id     TEXT NOT NULL REFERENCES blobs(id) ON DELETE CASCADE,
    added_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_enc_gallery_items ON encrypted_gallery_items(gallery_id);

-- Add encryption_mode flag to blobs table to track which are from encrypted galleries
-- vs global encryption. NULL or 'global' = part of global encryption toggle.
-- 'gallery' = belongs to an encrypted gallery (exempt from toggle).
-- Use a default so existing blobs are 'global'.
