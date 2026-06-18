-- Migration 023: Fix face/pet cluster photo_count to count DISTINCT photos.
--
-- The clustering pass historically stored photo_count as the number of face
-- (or pet) *detections* in a cluster, not the number of distinct photos. A
-- single image that contains many crops of the same person — a collage, a
-- movie-poster montage, a contact sheet — therefore inflated the People/Pets
-- card count (e.g. "33 photos" for a person whose album holds only 6).
--
-- The processor now recomputes this correctly on every clustering pass, but
-- clustering only re-runs for users with newly unclustered detections, so
-- existing libraries would keep the stale inflated values. Backfill them here.

UPDATE face_clusters
SET photo_count = (
    SELECT COUNT(DISTINCT fd.photo_id)
    FROM face_detections fd
    WHERE fd.cluster_id = face_clusters.id
);

UPDATE pet_clusters
SET photo_count = (
    SELECT COUNT(DISTINCT pd.photo_id)
    FROM pet_detections pd
    WHERE pd.cluster_id = pet_clusters.id
);
