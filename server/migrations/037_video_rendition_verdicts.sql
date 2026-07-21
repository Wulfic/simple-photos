-- Terminal verdicts for the video resolution ladder (#49).
--
-- `035` gave a rendition a place to live and `036` gave the generation pass a
-- way to give up. This gives it a way to say "nothing is owed here" — which is
-- a different answer from both "produced" and "failed", and without it the
-- pass mislabels a large, entirely healthy part of the library as broken.
--
-- Why a third state is required
-- ─────────────────────────────────────────────────────────────────────────
-- The candidate prefilter is deliberately WIDER than the ladder rule, because
-- 58 of the live library's 698 videos have no recorded geometry at all
-- (`photos.width`/`height` <= 0) and a prefilter keyed on `MIN(w,h) > 1080`
-- cannot see them. They are selected blind and resolved by an ffprobe — and
-- most of them turn out to need no rung.
--
-- With only the two states `036` provides, that verdict has nowhere to go. The
-- row keeps both locators NULL, which the candidate query reads as "still
-- owed", so the photo is re-selected on the next sweep, re-probed, and after
-- three passes retired by the attempt cap — logging a warning that says the
-- video "will not get a quality picker" when the truth is that it correctly
-- needs none. Three ffprobes and a false alarm per file, for a verdict that was
-- final the first time.
--
-- `not_needed` is therefore not an optimisation. It is the difference between
-- the attempt cap meaning "this file is broken" and meaning "this file is
-- either broken or fine, we no longer distinguish".
--
-- It is deliberately NOT a locator. A row carrying this flag is still
-- unplayable and `list_renditions` still filters it out, so no picker can ever
-- offer it, and the `036` nomination triggers (guarded on a locator) still do
-- not wake a single client to deliver a picker that has not changed.

ALTER TABLE video_renditions ADD COLUMN not_needed INTEGER NOT NULL DEFAULT 0;

-- ── A source rendition never owns its bytes ────────────────────────────────
-- The source rung points at the blob the PHOTO already has — it is a second
-- reference to existing bytes, not a new copy of them. `035`'s orphan trigger
-- was written before that row existed, and it cannot tell the two cases apart:
-- deleting a source rung queues the photo's own blob for collection.
--
-- The queue is only a hint and the (still unwritten) sweeper is required to
-- re-check references before unlinking, so this would not by itself destroy
-- data. But relying on that means the safety of a user's original 4K video
-- rests on a sweeper that does not exist yet being careful about a case its
-- author has to know about. Encoding the invariant in the trigger instead makes
-- it structural: only a rendition that owns its bytes can ever be queued, so
-- the sweeper cannot get this wrong even if it forgets.
DROP TRIGGER IF EXISTS trg_video_rendition_blob_orphaned;
CREATE TRIGGER trg_video_rendition_blob_orphaned
AFTER DELETE ON video_renditions
FOR EACH ROW
WHEN OLD.blob_id IS NOT NULL AND OLD.is_source = 0
BEGIN
    INSERT INTO orphaned_rendition_blobs (blob_id, user_id, detected_at)
    SELECT OLD.blob_id, b.user_id, datetime('now')
    FROM blobs b
    WHERE b.id = OLD.blob_id
    ON CONFLICT(blob_id) DO NOTHING;
END;
