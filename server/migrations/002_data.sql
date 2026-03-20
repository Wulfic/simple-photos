-- Photo & media storage tables.
--
-- Consolidated from: 004_encryption_modes (blobs/photos/galleries),
-- 010_favorites_and_crop, 011_trash_blob_support, 012_google_photos_metadata,
-- 013_camera_model, 014_photo_hashes, 015_encrypted_thumb_blob_id,
-- 005_trash_and_backup (trash_items), 009_photo_tags, 017_edit_copies,
-- 020_trash_metadata_preservation.
--
-- All photos are always encrypted (AES-256-GCM, client-side).
-- encrypted_blob_id is NOT NULL — there is no plain/unencrypted mode.

-- ── Blobs ───────────────────────────────────────────────────────────────────
CREATE TABLE blobs (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blob_type    TEXT NOT NULL,
    size_bytes   INTEGER NOT NULL,
    client_hash  TEXT,
    upload_time  TEXT NOT NULL,
    storage_path TEXT NOT NULL,
    content_hash TEXT
);
CREATE INDEX idx_blobs_user_type_time ON blobs(user_id, blob_type, upload_time);
CREATE INDEX idx_blobs_client_hash    ON blobs(user_id, client_hash) WHERE client_hash IS NOT NULL;
CREATE INDEX idx_blobs_content_hash   ON blobs(user_id, content_hash) WHERE content_hash IS NOT NULL;
CREATE INDEX idx_blobs_user_size      ON blobs(user_id, size_bytes);

-- ── Photos ──────────────────────────────────────────────────────────────────
CREATE TABLE photos (
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
CREATE INDEX idx_photos_user          ON photos(user_id, created_at);
CREATE INDEX idx_photos_file_path     ON photos(file_path);
CREATE INDEX idx_photos_taken_at      ON photos(user_id, taken_at);
CREATE INDEX idx_photos_encrypted_blob ON photos(encrypted_blob_id);
CREATE INDEX idx_photos_user_favorite ON photos(user_id, is_favorite) WHERE is_favorite = 1;
CREATE INDEX idx_photos_hash          ON photos(photo_hash);

CREATE UNIQUE INDEX idx_photos_user_hash
    ON photos(user_id, photo_hash) WHERE photo_hash IS NOT NULL;

-- Unique on user+file_path for non-empty paths (scan dedup)
CREATE UNIQUE INDEX idx_photos_unique_file_path
    ON photos(user_id, file_path) WHERE file_path != '';

-- Encrypted sync: ordered by display date
CREATE INDEX idx_photos_encrypted_sync
    ON photos(user_id, COALESCE(taken_at, created_at) DESC, filename ASC);

-- Composite for main list query
CREATE INDEX idx_photos_user_taken_or_created
    ON photos(user_id, COALESCE(taken_at, created_at) DESC, filename ASC);

-- ── Photo Metadata (Google Photos import, etc.) ────────────────────────────
CREATE TABLE photo_metadata (
    id             TEXT PRIMARY KEY,
    user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    photo_id       TEXT,
    blob_id        TEXT,
    source         TEXT NOT NULL DEFAULT 'manual',
    title          TEXT,
    description    TEXT,
    taken_at       TEXT,
    created_at_src TEXT,
    latitude       REAL,
    longitude      REAL,
    altitude       REAL,
    image_views    INTEGER,
    original_url   TEXT,
    storage_path   TEXT,
    imported_at    TEXT NOT NULL
);
CREATE INDEX idx_photo_metadata_user   ON photo_metadata(user_id);
CREATE INDEX idx_photo_metadata_photo  ON photo_metadata(photo_id) WHERE photo_id IS NOT NULL;
CREATE INDEX idx_photo_metadata_blob   ON photo_metadata(blob_id) WHERE blob_id IS NOT NULL;
CREATE INDEX idx_photo_metadata_source ON photo_metadata(user_id, source);

-- ── Photo Tags ──────────────────────────────────────────────────────────────
CREATE TABLE photo_tags (
    photo_id   TEXT NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tag        TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (photo_id, user_id, tag)
);
CREATE INDEX idx_photo_tags_user_tag ON photo_tags(user_id, tag);
CREATE INDEX idx_photo_tags_photo    ON photo_tags(photo_id, user_id);
CREATE INDEX idx_photo_tags_user     ON photo_tags(user_id);

-- ── Edit Copies ─────────────────────────────────────────────────────────────
CREATE TABLE edit_copies (
    id            TEXT PRIMARY KEY,
    photo_id      TEXT NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    edit_metadata TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX idx_edit_copies_photo ON edit_copies(photo_id, user_id);

-- ── Trash ───────────────────────────────────────────────────────────────────
CREATE TABLE trash_items (
    id                      TEXT PRIMARY KEY,
    user_id                 TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    photo_id                TEXT NOT NULL,
    filename                TEXT NOT NULL,
    file_path               TEXT NOT NULL,
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
    deleted_at              TEXT NOT NULL,
    expires_at              TEXT NOT NULL,
    encrypted_blob_id       TEXT,
    thumbnail_blob_id       TEXT,
    is_favorite             INTEGER NOT NULL DEFAULT 0,
    crop_metadata           TEXT,
    camera_model            TEXT,
    photo_hash              TEXT,
    encrypted_thumb_blob_id TEXT,
    client_hash             TEXT,
    content_hash            TEXT
);
CREATE INDEX idx_trash_user    ON trash_items(user_id, deleted_at);
CREATE INDEX idx_trash_expires ON trash_items(expires_at);
