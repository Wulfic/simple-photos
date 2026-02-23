-- Shared albums: server-side albums that can be shared between users.
-- Unlike encrypted blob-based albums (client-side), shared albums are
-- stored in plain on the server so multiple users can access them.

CREATE TABLE shared_albums (
    id            TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_shared_albums_owner ON shared_albums(owner_user_id);

CREATE TABLE shared_album_members (
    id        TEXT PRIMARY KEY,
    album_id  TEXT NOT NULL REFERENCES shared_albums(id) ON DELETE CASCADE,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    added_at  TEXT NOT NULL,
    UNIQUE(album_id, user_id)
);
CREATE INDEX idx_shared_album_members_user ON shared_album_members(user_id);
CREATE INDEX idx_shared_album_members_album ON shared_album_members(album_id);

-- Photos in a shared album reference either a plain-mode photo or a blob id.
-- For plain-mode photos we reference photos.id; for encrypted we reference blobs.id.
-- Using a generic `photo_ref` + `ref_type` approach.
CREATE TABLE shared_album_photos (
    id        TEXT PRIMARY KEY,
    album_id  TEXT NOT NULL REFERENCES shared_albums(id) ON DELETE CASCADE,
    photo_ref TEXT NOT NULL,  -- photos.id or blobs.id depending on mode
    ref_type  TEXT NOT NULL DEFAULT 'plain',  -- 'plain' or 'blob'
    added_at  TEXT NOT NULL,
    UNIQUE(album_id, photo_ref)
);
CREATE INDEX idx_shared_album_photos_album ON shared_album_photos(album_id);
