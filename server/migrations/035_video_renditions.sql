-- Resolution ladder for video (#49).
--
-- A source above the 1080p tier gets a second, smaller rendition so the player
-- can offer a quality choice, and so a client on a slow link is not forced to
-- pull a 4K file. Measured demand on the live library: 136 of 742 videos, and
-- 126 of those are 3840x2160. Rung selection is `transcode::ladder`; this file
-- only stores the results.
--
-- Why renditions are blobs, not just files on disk
-- ─────────────────────────────────────────────────────────────────────────
-- The obvious design — a rendition is a file, served by a query parameter on
-- `GET /photos/:id/file` — cannot work, because neither client plays from that
-- route. The web viewer downloads the whole encrypted blob and decrypts it
-- (`useViewerMedia.loadEncryptedMedia`); Android range-decrypts a blob through
-- `MediaBlobDataSource` (`spblob://`). A rendition that exists only as a
-- plaintext file on disk is invisible to both. So `blob_id` is the column that
-- matters in encrypted mode.
--
-- `file_path` is kept alongside it because the server also runs unencrypted
-- (`photos.encrypted_blob_id` is nullable and 2,494 live rows have it NULL).
-- The pair mirrors `photos.file_path` / `photos.encrypted_blob_id` exactly,
-- deliberately: a rendition is storable in whichever mode its parent photo is.
-- Exactly one of the two is expected to be set; neither being set means the
-- rendition is planned but not yet produced.

CREATE TABLE IF NOT EXISTS video_renditions (
    -- Cascade is correct here, unlike `photo_change_log`: a rendition describes
    -- a photo's *current* content and has no reason to outlive it. The bytes it
    -- points at are cleaned up by the GC queue below, which the cascade itself
    -- populates.
    photo_id   TEXT    NOT NULL REFERENCES photos(id) ON DELETE CASCADE,

    -- Short edge — `min(width, height)`, the ladder's identity for a rung.
    --
    -- NOT height. 14 videos in the live library are exactly 1080x1920 and 4 are
    -- 2288x1088; keyed on height, the first group collides with a genuine 1080p
    -- rung and the second invents one 8 pixels tall. See `transcode::ladder`.
    short_edge INTEGER NOT NULL,

    width      INTEGER NOT NULL,
    height     INTEGER NOT NULL,

    -- Whether this row is the untouched source rather than a downscale.
    --
    -- A source rung is only offerable if the source itself is playable — #46
    -- established that a `.mp4` may be HEVC, 10-bit, or a corrupt bitstream
    -- behind an intact container. A ladder whose top rung is one of those hands
    -- the user a picker whose "highest" option does not play, which is a worse
    -- bug than having no picker.
    is_source  INTEGER NOT NULL DEFAULT 0,

    -- Encrypted mode: the blob a client downloads / range-decrypts.
    blob_id    TEXT,
    -- Unencrypted mode: storage-root-relative path, forward slashes.
    file_path  TEXT,

    codec      TEXT,
    bitrate    INTEGER,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT    NOT NULL,

    -- One rendition per rung per photo. Re-running the ladder must update a
    -- rung, never accumulate duplicates of it.
    PRIMARY KEY (photo_id, short_edge)
);

-- The picker reads every rung for one photo, highest first.
CREATE INDEX IF NOT EXISTS idx_video_renditions_photo
    ON video_renditions (photo_id, short_edge DESC);

-- Resolving a blob back to the rendition that owns it — used by the GC sweep
-- and by serving, which must confirm a requested blob really is a rendition of
-- a photo the caller may read.
CREATE INDEX IF NOT EXISTS idx_video_renditions_blob
    ON video_renditions (blob_id) WHERE blob_id IS NOT NULL;

-- ── Orphaned rendition bytes ───────────────────────────────────────────────
-- Deleting a photo cascades its rendition rows away, but a DB cascade cannot
-- unlink a file or drop the `blobs` row. Doing that at the call sites is not an
-- option: `DELETE FROM photos` happens at nine sites across seven files (see
-- `033_photo_change_log.sql`), and a missed one here leaks a *video-sized*
-- blob — for the 4K sources this feature targets, hundreds of megabytes each,
-- forever, invisibly.
--
-- Same resolution as #38's tombstones, for the same reason: one trigger covers
-- every present and future delete site, and the row it writes is a HINT ("these
-- bytes are probably unreferenced") rather than a claim. The sweeper re-checks
-- that nothing references the blob before unlinking, so an over-fired trigger
-- costs one wasted lookup and can never delete live data.
--
-- Verified in #38: SQLite fires AFTER DELETE triggers for rows removed by
-- ON DELETE CASCADE, which is the entire reason this works from a photo delete.
CREATE TABLE IF NOT EXISTS orphaned_rendition_blobs (
    blob_id     TEXT PRIMARY KEY,
    -- Denormalised: the blob's path is derived from (user_id, blob_id), and the
    -- photo row that could supply user_id is already gone by the time the
    -- sweeper runs.
    user_id     TEXT NOT NULL,
    detected_at TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS trg_video_rendition_blob_orphaned
AFTER DELETE ON video_renditions
FOR EACH ROW
WHEN OLD.blob_id IS NOT NULL
BEGIN
    INSERT INTO orphaned_rendition_blobs (blob_id, user_id, detected_at)
    SELECT OLD.blob_id, b.user_id, datetime('now')
    FROM blobs b
    WHERE b.id = OLD.blob_id
    ON CONFLICT(blob_id) DO NOTHING;
END;

-- ── Delta-feed integration ─────────────────────────────────────────────────
-- Renditions arrive asynchronously, minutes to hours after the photo itself:
-- the 1080p rung is deliberately enqueued behind first-pass conversions so no
-- one waits on it to see their video at all. By then every client has long
-- since synced that photo and will not ask about it again — the #38 delta feed
-- only re-sends what `photo_change_log` nominates.
--
-- Without this trigger the picker would stay empty until the client's next full
-- walk, which after #38 may be never. Nominating the parent photo is exactly
-- what the hint log is for: it does not assert what changed, it only asks the
-- reader to look again.
CREATE TRIGGER IF NOT EXISTS trg_photo_change_log_rendition_insert
AFTER INSERT ON video_renditions
FOR EACH ROW
BEGIN
    INSERT INTO photo_change_log (photo_id, user_id, seq, changed_at)
    SELECT
        p.id,
        p.user_id,
        (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        datetime('now')
    FROM photos p
    WHERE p.id = NEW.photo_id
    ON CONFLICT(photo_id) DO UPDATE SET
        seq        = (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        changed_at = excluded.changed_at;
END;

-- A rendition being withdrawn (failed re-encode cleaned up, ladder re-planned)
-- changes what the picker may offer just as much as one appearing. Guarded on
-- the photo still existing: when the cascade is what deleted this row, the
-- photo is already gone and `trg_photo_change_log_delete` has written the real
-- tombstone — the SELECT below simply matches nothing, so the two do not fight.
CREATE TRIGGER IF NOT EXISTS trg_photo_change_log_rendition_delete
AFTER DELETE ON video_renditions
FOR EACH ROW
BEGIN
    INSERT INTO photo_change_log (photo_id, user_id, seq, changed_at)
    SELECT
        p.id,
        p.user_id,
        (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        datetime('now')
    FROM photos p
    WHERE p.id = OLD.photo_id
    ON CONFLICT(photo_id) DO UPDATE SET
        seq        = (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        changed_at = excluded.changed_at;
END;

-- No backfill. Unlike `033`, an absent row here is not a gap in a sequence —
-- it means "this photo has no ladder yet", which is the correct state for every
-- existing video until the opt-in backfill task is run against it. Seeding
-- source rows for 742 videos would claim renditions that do not exist on disk.
