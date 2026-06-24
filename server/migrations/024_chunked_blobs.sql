-- Chunked (v2) encrypted blob format + per-photo encryption deferral.
--
-- Large videos used to OOM-abort the server during the encrypt-after-convert
-- phase: the v1 blob format base64-wraps the whole file in a JSON envelope and
-- encrypts it as one AES-GCM message, holding ~5× the file size in RAM at once.
-- The v2 format streams the file as independent length-prefixed AES-GCM chunk
-- frames (see blobs/chunked.rs), bounding memory to one chunk.
--
-- `blob_format` records which container a blob uses so the server and clients
-- pick the right decrypt path. NULL/1 = legacy monolithic envelope, 2 = chunked.
-- Existing blobs predate the chunked path, so the default of 1 is correct.
ALTER TABLE blobs ADD COLUMN blob_format INTEGER NOT NULL DEFAULT 1;

-- Defer marker: if a photo genuinely cannot be encrypted (e.g. an
-- implausibly large file even for the chunked path, or a repeated hard
-- failure), we set `encryption_deferred = 1` and record `encryption_error`
-- so the migration loop skips it instead of retrying forever and re-crashing.
-- Surfaced to the import UI; the photo stays unencrypted until the cause is
-- resolved.
ALTER TABLE photos ADD COLUMN encryption_deferred INTEGER NOT NULL DEFAULT 0;
ALTER TABLE photos ADD COLUMN encryption_error TEXT;

-- Failed-attempt counter. After MIGRATION_MAX_ATTEMPTS hard failures the photo
-- is flipped to encryption_deferred = 1 so the migration loop stops retrying it
-- (and, before the chunked path, stops re-triggering the OOM abort).
ALTER TABLE photos ADD COLUMN encryption_attempts INTEGER NOT NULL DEFAULT 0;

-- The migration / pipeline-busy queries filter on this to skip deferred photos
-- without a full table scan.
CREATE INDEX IF NOT EXISTS idx_photos_enc_pending
    ON photos(encrypted_blob_id, encryption_deferred);
