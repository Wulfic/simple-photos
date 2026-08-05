-- Attempt accounting for the video resolution ladder (#49).
--
-- `035` gave a rendition a place to live. This gives the generation pass a way
-- to give up.
--
-- Why this is not optional
-- ─────────────────────────────────────────────────────────────────────────
-- The candidate set is "videos whose short edge exceeds the 1080p tier and
-- which have no rung yet". That query is self-limiting on success — a produced
-- rung removes its photo from the set forever. On *failure* it is not: the
-- photo stays a candidate, and the pass re-attempts a 4K-class re-encode on
-- every sweep, forever. 114 live candidates, 26 of them 3840x2160 and 4 of them
-- 8K, is not a queue anyone wants stuck in a loop.
--
-- Same shape as the 3-strike cap `todo.md` plans for conversion (#40), and
-- deliberately so: `scan_skipped_paths` (031) already established that the way
-- to stop re-doing failed work in this codebase is to record the attempt, not
-- to remember it in memory.
--
-- The counter is incremented BEFORE the encode runs, not after it fails. A file
-- that reliably OOMs or hard-kills ffmpeg never reaches an error handler, and
-- that is precisely the file most in need of retirement. Counting attempts
-- rather than failures is what makes the cap hold against a crash.

ALTER TABLE video_renditions ADD COLUMN attempt_count   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE video_renditions ADD COLUMN last_error      TEXT;
ALTER TABLE video_renditions ADD COLUMN last_attempt_at TEXT;

-- ── Nomination triggers, corrected ─────────────────────────────────────────
-- `035` nominates the parent photo for re-sync whenever a rendition row is
-- INSERTed, so the #38 delta feed re-sends a photo whose picker gained an
-- option. Attempt accounting breaks two assumptions that design rested on, and
-- both were verified against SQLite rather than reasoned about:
--
-- 1. **An upsert that takes the DO UPDATE branch fires UPDATE triggers, not
--    INSERT triggers.** `upsert_rendition` is `INSERT ... ON CONFLICT DO
--    UPDATE`, so the moment a rung stops being a claim and becomes playable is
--    an UPDATE — and `035` has no UPDATE trigger. The photo would never be
--    nominated, and the picker would stay empty until a full walk that, after
--    #38, may never come. This already affects any re-encode of an existing
--    rung; attempt accounting merely makes it the normal path.
--
-- 2. **A claim row is not worth telling a client about.** With attempts
--    recorded, rows now exist that have no `blob_id` and no `file_path` —
--    `035` calls that state "planned but not produced" and `list_renditions`
--    filters it out. Nominating on those would wake every client once per
--    attempt to deliver a picker that has not changed.
--
-- So the rule becomes precise: nominate exactly when the set of *playable*
-- renditions changes. Bumping `attempt_count` on a failure is invisible to
-- clients and must stay that way.

DROP TRIGGER IF EXISTS trg_photo_change_log_rendition_insert;
CREATE TRIGGER trg_photo_change_log_rendition_insert
AFTER INSERT ON video_renditions
FOR EACH ROW
-- Only a row with bytes behind it changes what a picker may offer.
WHEN NEW.blob_id IS NOT NULL OR NEW.file_path IS NOT NULL
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

-- The case `035` could not have: a claim row acquiring its bytes, or a rung
-- being re-encoded onto a different blob. `IS NOT` rather than `!=` because
-- either side may be NULL, and NULL != 'b1' is NULL — which a WHEN clause
-- treats as false, silently skipping the nomination this trigger exists for.
CREATE TRIGGER trg_photo_change_log_rendition_update
AFTER UPDATE ON video_renditions
FOR EACH ROW
WHEN NEW.blob_id IS NOT OLD.blob_id OR NEW.file_path IS NOT OLD.file_path
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

-- Symmetric with the INSERT guard: withdrawing a rung a client could never see
-- tells that client nothing. The unguarded `035` version woke every client when
-- a failed claim row was cleaned up.
DROP TRIGGER IF EXISTS trg_photo_change_log_rendition_delete;
CREATE TRIGGER trg_photo_change_log_rendition_delete
AFTER DELETE ON video_renditions
FOR EACH ROW
WHEN OLD.blob_id IS NOT NULL OR OLD.file_path IS NOT NULL
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

-- Finding the work. Partial index on the rows that are still owed bytes, which
-- is the only thing the candidate query joins against.
CREATE INDEX IF NOT EXISTS idx_video_renditions_pending
    ON video_renditions (photo_id, short_edge, attempt_count)
    WHERE blob_id IS NULL AND file_path IS NULL;
