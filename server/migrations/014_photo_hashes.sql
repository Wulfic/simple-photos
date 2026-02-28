-- 014: Content-based photo hashes for cross-platform alignment.
-- Each photo/blob gets a short deterministic hash (12 hex chars from SHA-256 of
-- the original file content).  This guarantees web ↔ app photo identity regardless
-- of upload order or server-assigned UUIDs.

-- Plain-mode photos: server computes hash from raw file bytes.
ALTER TABLE photos ADD COLUMN photo_hash TEXT;

-- Encrypted blobs: client computes hash of raw bytes *before* encryption,
-- sends via X-Content-Hash header.
ALTER TABLE blobs ADD COLUMN content_hash TEXT;

-- Master index — one hash per user per unique photo.
CREATE UNIQUE INDEX idx_photos_user_hash
    ON photos(user_id, photo_hash)
    WHERE photo_hash IS NOT NULL;

CREATE INDEX idx_blobs_content_hash
    ON blobs(user_id, content_hash)
    WHERE content_hash IS NOT NULL;
