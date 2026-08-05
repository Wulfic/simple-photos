-- Migration 039: Pet detection bounding boxes
--
-- `face_detections` has carried bbox_x/y/w/h since migration 017, which is what
-- lets the People tile crop to the face (#48). `pet_detections` (migration 020)
-- never had them, so Pet tiles are circular but centre-cropped and no amount of
-- client work can fix that — the number simply is not in the database.
--
-- Adding the columns only helps photos processed from here on. The backfill
-- below is what makes the feature work on an existing library, and it is exact
-- rather than a guess:
--
--   `pet_detections` rows are DERIVED from `object_detections` rows. The AI
--   processor iterates the object detections, maps `class_name` to a species,
--   deduplicates by species within the photo, and copies that detection's
--   `confidence` verbatim into the pet row. `object_detections` has stored the
--   bbox all along. So the pet row's bbox is recoverable by joining back to the
--   object detection it was made from — no re-running the models over the
--   library, which on a 15k-photo library is hours of GPU.
--
-- The species map below mirrors `PET_SPECIES` in `server/src/ai/animal/mod.rs`.
-- It is many-to-one ('big cat' folds into 'cat'), which is why the join maps the
-- class name rather than comparing it to `species` directly. Keep the two in
-- step: a species added there and not here silently backfills nothing, which
-- degrades to the pre-migration behaviour (NULL bbox, plain crop) rather than
-- producing a wrong box.

ALTER TABLE pet_detections ADD COLUMN bbox_x REAL;
ALTER TABLE pet_detections ADD COLUMN bbox_y REAL;
ALTER TABLE pet_detections ADD COLUMN bbox_w REAL;
ALTER TABLE pet_detections ADD COLUMN bbox_h REAL;

-- Recover the originating object detection's box, in two passes.
--
-- Pass 1 requires an exact `confidence` match. The processor copied that value
-- through unchanged (same REAL column, no arithmetic), so it identifies the
-- precise source row even when one photo holds several detections of the same
-- species — e.g. a 'cat' and a 'big cat', which both fold to species 'cat'.
--
-- Pass 2 then sweeps whatever pass 1 left NULL, taking the highest-confidence
-- detection of the right species on the same photo. That is still a correct box
-- for that species in that photo, so a confidence mismatch degrades the framing
-- rather than losing it.
--
-- This is two statements rather than one `ORDER BY (od.confidence =
-- pet_detections.confidence) DESC` because SQLite resolves the UPDATE target's
-- columns inside a subquery's WHERE but NOT inside its ORDER BY — the one-pass
-- form fails to prepare with "no such column: pet_detections.confidence".
-- Both passes share the species map above; edit them together.

-- Pass 1 — exact source row.
UPDATE pet_detections
SET (bbox_x, bbox_y, bbox_w, bbox_h) = (
    SELECT od.bbox_x, od.bbox_y, od.bbox_w, od.bbox_h
    FROM object_detections od
    WHERE od.photo_id = pet_detections.photo_id
      AND od.user_id = pet_detections.user_id
      AND od.confidence = pet_detections.confidence
      AND CASE od.class_name
            WHEN 'dog'     THEN 'dog'
            WHEN 'cat'     THEN 'cat'
            WHEN 'big cat' THEN 'cat'
            WHEN 'bird'    THEN 'bird'
            WHEN 'horse'   THEN 'horse'
            WHEN 'rabbit'  THEN 'rabbit'
            WHEN 'hamster' THEN 'hamster'
            WHEN 'fish'    THEN 'fish'
          END = pet_detections.species
    ORDER BY od.confidence DESC
    LIMIT 1
)
WHERE bbox_x IS NULL;

-- Pass 2 — best remaining detection of the same species on the same photo.
UPDATE pet_detections
SET (bbox_x, bbox_y, bbox_w, bbox_h) = (
    SELECT od.bbox_x, od.bbox_y, od.bbox_w, od.bbox_h
    FROM object_detections od
    WHERE od.photo_id = pet_detections.photo_id
      AND od.user_id = pet_detections.user_id
      AND CASE od.class_name
            WHEN 'dog'     THEN 'dog'
            WHEN 'cat'     THEN 'cat'
            WHEN 'big cat' THEN 'cat'
            WHEN 'bird'    THEN 'bird'
            WHEN 'horse'   THEN 'horse'
            WHEN 'rabbit'  THEN 'rabbit'
            WHEN 'hamster' THEN 'hamster'
            WHEN 'fish'    THEN 'fish'
          END = pet_detections.species
    ORDER BY od.confidence DESC
    LIMIT 1
)
WHERE bbox_x IS NULL;
