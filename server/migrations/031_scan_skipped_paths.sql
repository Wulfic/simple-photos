-- Remember scan candidates the register pass already rejected, so the
-- background auto-scan stops re-hashing them over the (often SMB) storage mount
-- on every interval tick.
--
-- The bug this fixes: Google Takeout stores identical bytes in BOTH the
-- "Photos from YYYY" date folder AND every album folder a photo belongs to. The
-- date-folder copy registers first; every album copy then collides on the
-- (user_id, photo_hash) unique index and is a dedup no-op. The auto-scan walk
-- only skipped paths already present in photos.file_path / source_path / trash,
-- so these album copies were re-walked, re-EXIF'd and — worst — re-hashed (a
-- full-file streaming SHA over the storage mount) on EVERY pass, forever. A live
-- box measured 4,254 such files re-processed every 300s (~130s of grinding per
-- tick, a 43% disk-I/O duty cycle at idle). The same hole applied to files whose
-- content hash matches a gallery-hidden (secure-gallery) original.
--
-- This table is a pure CACHE of "already examined, nothing to do". It is only
-- ever written by the register pass (crate::photos::register) and read by the
-- walk (crate::backup::autoscan). A missing or stale row only ever costs one
-- extra re-hash of that single file, never correctness — the file is simply
-- re-evaluated and, if still a duplicate / still gallery-hidden, re-recorded.
-- That property is what makes the generous, trigger-based invalidation below
-- safe: an over-eager delete here can never lose or hide a photo.
CREATE TABLE IF NOT EXISTS scan_skipped_paths (
    -- Owner the candidate would have been registered to (the scan's admin user).
    user_id     TEXT NOT NULL,
    -- Storage-root-relative path with forward slashes — the same key the walk
    -- derives for every file, matched directly against this row.
    rel_path    TEXT NOT NULL,
    -- Size + mtime captured when we rejected it. If EITHER changes, the file was
    -- replaced/edited on disk and must be re-evaluated (the walk drops the stale
    -- row and reprocesses), so a re-dropped or modified file is never wrongly
    -- skipped.
    size_bytes  INTEGER NOT NULL,
    mtime       TEXT,
    -- Why it was skipped: 'hash_duplicate' (collided on the photo_hash unique
    -- index) or 'gallery_hidden' (content matches a secure-gallery original).
    reason      TEXT NOT NULL,
    -- The content hash that drove the skip. Drives invalidation: when the photo
    -- (or secure-gallery item) carrying this hash is deleted, this row must go so
    -- a copy still on disk can register again — exactly what the scan would do
    -- without this cache.
    photo_hash  TEXT,
    created_at  TEXT NOT NULL,
    PRIMARY KEY (user_id, rel_path)
);

-- The delete-triggers below invalidate by hash; index it so those deletes (and
-- an empty-trash of thousands of rows) stay cheap.
CREATE INDEX IF NOT EXISTS idx_scan_skipped_paths_hash
    ON scan_skipped_paths (photo_hash);

-- Invalidation is fully automatic and future-proof. The skip cache must never
-- change observable scan behaviour: without it, the scan re-registers any
-- on-disk file that isn't currently in `photos`. So the moment the photo a copy
-- deduped against is deleted — a trash purge, a secure-gallery move, a
-- disaster-recovery reconcile, ANY of the ~10 delete sites — its skip rows must
-- drop so the copy is re-evaluated. A trigger covers every site at once (present
-- and future) with zero code at the call sites, and the "re-hash is the only
-- cost" property makes firing it too often harmless.
CREATE TRIGGER IF NOT EXISTS trg_scan_skipped_photo_delete
AFTER DELETE ON photos
FOR EACH ROW
WHEN OLD.photo_hash IS NOT NULL
BEGIN
    DELETE FROM scan_skipped_paths WHERE photo_hash = OLD.photo_hash;
END;

-- Same, for the gallery-hidden case: removing a secure-gallery item un-hides its
-- content hash, so a matching file on disk should be allowed back into the main
-- gallery on the next scan.
CREATE TRIGGER IF NOT EXISTS trg_scan_skipped_egi_delete
AFTER DELETE ON encrypted_gallery_items
FOR EACH ROW
WHEN OLD.original_photo_hash IS NOT NULL
BEGIN
    DELETE FROM scan_skipped_paths WHERE photo_hash = OLD.original_photo_hash;
END;
