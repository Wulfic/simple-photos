-- Delta-sync change log for the encrypted-sync feed (#38).
--
-- The problem: every client rebuilt its entire mirror on every sync pass,
-- because the server had no way to answer "what changed since I last looked?".
-- A full walk of a 15k-row library, every pass, forever.
--
-- Why a naive delta sync would have been a REGRESSION, not a fix
-- ─────────────────────────────────────────────────────────────────────────
-- Today's full walk is self-healing: the client set-differences what it
-- received against what it holds, so any row the server stops returning is
-- dropped locally no matter WHY it stopped being returned. A delta feed gives
-- that up — it must name every departure explicitly, and anything it fails to
-- name stays in every client forever. Two properties of this schema make that
-- unusually easy to get wrong:
--
--   1. `DELETE FROM photos` happens at nine call sites across seven files, and
--      more arrive with every feature.
--   2. Eligibility for the feed is not a column. It is a subquery against
--      `encrypted_gallery_items` — so moving a photo into a secure album makes
--      it vanish from sync WITHOUT the photo row being touched at all. A
--      trigger watching only `photos` would never see it.
--
-- The design that makes it safe: this log is a HINT, not a source of truth
-- ─────────────────────────────────────────────────────────────────────────
-- A row here means only "something about this photo may have changed" — never
-- "this photo is deleted" or "this photo is eligible". The delta query in
-- `gallery::sync` recomputes eligibility at read time from the SAME predicate
-- the full walk uses (`gallery::eligibility::ELIGIBLE_PREDICATE`), and derives
-- the upsert/tombstone split from that. Consequences, deliberately:
--
--   * A trigger that fires too often costs one redundant row in one delta
--     page. Harmless. Fire freely.
--   * A trigger cannot report a photo as deleted when it is not, or as
--     eligible when it is not, because it does not report either. It only
--     nominates candidates for re-examination.
--
-- That inverts the usual tombstone risk. Compare `031_scan_skipped_paths.sql`:
-- that trigger is safe to over-fire because a stray delete only costs a
-- re-hash. Same reasoning, same conclusion — and here it buys us immunity to
-- the nine-delete-sites problem instead of just cheap invalidation.
--
-- The remaining risk is a write path that bypasses triggers entirely (a
-- restore that loads a DB file wholesale, say). `photos_summary` therefore
-- also returns `head_seq` and `total` so a client can cheaply detect that its
-- mirror has drifted and fall back to a full walk. Missed changes degrade to
-- "stale until the next integrity check", never "silently wrong forever".

CREATE TABLE IF NOT EXISTS photo_change_log (
    -- Deliberately NOT a foreign key to photos(id). The whole point of a
    -- tombstone is to outlive the row it describes; an FK with ON DELETE
    -- CASCADE would delete the evidence at the exact moment it starts
    -- mattering, and the delta feed would never mention the deletion.
    photo_id   TEXT PRIMARY KEY,

    -- Denormalised so tombstones remain attributable to a user after the
    -- photo row (and its user_id) is gone. Delta queries are per-user.
    user_id    TEXT NOT NULL,

    -- Monotonic change counter, global across users rather than per-user.
    -- Global is simpler and still correct: a client's cursor only ever needs
    -- to be comparable with itself, and per-user sequences would need their
    -- own bookkeeping table to stay gapless. SQLite has a single writer, so
    -- MAX(seq)+1 inside a trigger cannot race.
    seq        INTEGER NOT NULL,

    changed_at TEXT NOT NULL
);

-- Drives both MAX(seq) (head sequence, and the +1 every trigger below
-- computes) and the `seq > ?` delta scan.
CREATE INDEX IF NOT EXISTS idx_photo_change_log_seq
    ON photo_change_log (seq);
CREATE INDEX IF NOT EXISTS idx_photo_change_log_user_seq
    ON photo_change_log (user_id, seq);

-- ── photos: the row itself changed ─────────────────────────────────────────
-- One row per photo, its seq bumped in place, so the log stays bounded by the
-- library size instead of growing without limit. A client that misses many
-- changes to one photo just re-fetches it once.

CREATE TRIGGER IF NOT EXISTS trg_photo_change_log_insert
AFTER INSERT ON photos
FOR EACH ROW
BEGIN
    INSERT INTO photo_change_log (photo_id, user_id, seq, changed_at)
    VALUES (
        NEW.id,
        NEW.user_id,
        (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        datetime('now')
    )
    ON CONFLICT(photo_id) DO UPDATE SET
        seq        = (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        user_id    = excluded.user_id,
        changed_at = excluded.changed_at;
END;

CREATE TRIGGER IF NOT EXISTS trg_photo_change_log_update
AFTER UPDATE ON photos
FOR EACH ROW
BEGIN
    INSERT INTO photo_change_log (photo_id, user_id, seq, changed_at)
    VALUES (
        NEW.id,
        NEW.user_id,
        (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        datetime('now')
    )
    ON CONFLICT(photo_id) DO UPDATE SET
        seq        = (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        user_id    = excluded.user_id,
        changed_at = excluded.changed_at;
END;

-- The tombstone. Fires for all nine `DELETE FROM photos` sites at once, plus
-- any future one, plus the `users` ON DELETE CASCADE path.
CREATE TRIGGER IF NOT EXISTS trg_photo_change_log_delete
AFTER DELETE ON photos
FOR EACH ROW
BEGIN
    INSERT INTO photo_change_log (photo_id, user_id, seq, changed_at)
    VALUES (
        OLD.id,
        OLD.user_id,
        (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        datetime('now')
    )
    ON CONFLICT(photo_id) DO UPDATE SET
        seq        = (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        changed_at = excluded.changed_at;
END;

-- ── encrypted_gallery_items: eligibility changed, photo row did not ────────
-- The case a photos-only trigger cannot see. Adding a photo to a secure album
-- removes it from the feed; removing it from the album puts it back. Neither
-- writes to `photos`.
--
-- The three-way match mirrors ELIGIBLE_PREDICATE exactly: an item can name the
-- photo by its own id (`blob_id`), by the id it was cloned from
-- (`original_blob_id`), or indirectly via the photo's encrypted blob. Keep
-- these in sync — if the predicate grows a fourth arm, so do these triggers.
--
-- Verified: SQLite fires these for rows removed by ON DELETE CASCADE from
-- `encrypted_galleries` and `blobs`, so deleting a whole secure album
-- correctly un-hides its photos. See the `cascade` test in gallery::sync.

CREATE TRIGGER IF NOT EXISTS trg_photo_change_log_egi_insert
AFTER INSERT ON encrypted_gallery_items
FOR EACH ROW
BEGIN
    INSERT INTO photo_change_log (photo_id, user_id, seq, changed_at)
    SELECT
        p.id,
        p.user_id,
        (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        datetime('now')
    FROM photos p
    WHERE p.id = NEW.blob_id
       OR p.id = NEW.original_blob_id
       OR (NEW.original_blob_id IS NOT NULL
           AND p.encrypted_blob_id = NEW.original_blob_id)
    ON CONFLICT(photo_id) DO UPDATE SET
        seq        = (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        changed_at = excluded.changed_at;
END;

CREATE TRIGGER IF NOT EXISTS trg_photo_change_log_egi_delete
AFTER DELETE ON encrypted_gallery_items
FOR EACH ROW
BEGIN
    INSERT INTO photo_change_log (photo_id, user_id, seq, changed_at)
    SELECT
        p.id,
        p.user_id,
        (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        datetime('now')
    FROM photos p
    WHERE p.id = OLD.blob_id
       OR p.id = OLD.original_blob_id
       OR (OLD.original_blob_id IS NOT NULL
           AND p.encrypted_blob_id = OLD.original_blob_id)
    ON CONFLICT(photo_id) DO UPDATE SET
        seq        = (SELECT COALESCE(MAX(seq), 0) + 1 FROM photo_change_log),
        changed_at = excluded.changed_at;
END;

-- ── Backfill ───────────────────────────────────────────────────────────────
-- Seed one entry per existing photo so `?since=0` degenerates into exactly a
-- full sync rather than returning nothing. That means the delta endpoint needs
-- no special cold-start branch: a brand-new client and a client that has been
-- offline since before this migration take the identical code path.
--
-- ROW_NUMBER() gives distinct ascending sequences; the specific order is
-- irrelevant, only that every row is <= the head and the head is the count.
INSERT INTO photo_change_log (photo_id, user_id, seq, changed_at)
SELECT
    id,
    user_id,
    ROW_NUMBER() OVER (ORDER BY created_at, id),
    datetime('now')
FROM photos
WHERE true
ON CONFLICT(photo_id) DO NOTHING;
