-- Social & sharing tables.
--
-- Consolidated from: 004_encryption_modes (galleries), 007_shared_albums.
--
-- ref_type on shared_album_photos:
--   'photo' = references photos.id  (photos-table FK)
--   'blob'  = references blobs.id   (blobs-table FK)

-- ── Encrypted (Secure) Galleries ────────────────────────────────────────────
CREATE TABLE encrypted_galleries (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_enc_galleries_user ON encrypted_galleries(user_id);

CREATE TABLE encrypted_gallery_items (
    id         TEXT PRIMARY KEY,
    gallery_id TEXT NOT NULL REFERENCES encrypted_galleries(id) ON DELETE CASCADE,
    blob_id    TEXT NOT NULL REFERENCES blobs(id) ON DELETE CASCADE,
    added_at   TEXT NOT NULL
);
CREATE INDEX idx_enc_gallery_items ON encrypted_gallery_items(gallery_id);

-- ── Shared Albums ───────────────────────────────────────────────────────────
CREATE TABLE shared_albums (
    id            TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_shared_albums_owner ON shared_albums(owner_user_id);

CREATE TABLE shared_album_members (
    id       TEXT PRIMARY KEY,
    album_id TEXT NOT NULL REFERENCES shared_albums(id) ON DELETE CASCADE,
    user_id  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    added_at TEXT NOT NULL,
    UNIQUE(album_id, user_id)
);
CREATE INDEX idx_shared_album_members_user  ON shared_album_members(user_id);
CREATE INDEX idx_shared_album_members_album ON shared_album_members(album_id);

CREATE TABLE shared_album_photos (
    id        TEXT PRIMARY KEY,
    album_id  TEXT NOT NULL REFERENCES shared_albums(id) ON DELETE CASCADE,
    photo_ref TEXT NOT NULL,
    ref_type  TEXT NOT NULL DEFAULT 'photo',
    added_at  TEXT NOT NULL,
    UNIQUE(album_id, photo_ref)
);
CREATE INDEX idx_shared_album_photos_album ON shared_album_photos(album_id);
