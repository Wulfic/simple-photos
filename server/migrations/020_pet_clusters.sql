-- Migration 020: Pet Clustering
--
-- Adds tables for per-individual pet face matching:
-- pet_clusters holds one row per unique pet (like face_clusters holds one row
-- per unique person), and pet_detections records every photo where a pet was
-- identified along with its embedding vector for re-ID clustering.
--
-- Species is stored on both tables so UI can group / filter by "dog" vs "cat".

-- Pet identity clusters (one per recognised individual pet).
CREATE TABLE IF NOT EXISTS pet_clusters (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label           TEXT,                        -- user-assigned name, e.g. "Bubs"
    species         TEXT NOT NULL DEFAULT 'unknown',  -- "dog", "cat", etc.
    representative  TEXT,                        -- photo_id used as cover thumbnail
    photo_count     INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pet_cluster_user    ON pet_clusters(user_id);
CREATE INDEX IF NOT EXISTS idx_pet_cluster_species ON pet_clusters(user_id, species);
CREATE INDEX IF NOT EXISTS idx_pet_cluster_label   ON pet_clusters(user_id, label);

-- Per-photo pet detection records.
CREATE TABLE IF NOT EXISTS pet_detections (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id    TEXT NOT NULL REFERENCES photos(id)       ON DELETE CASCADE,
    user_id     TEXT NOT NULL REFERENCES users(id)        ON DELETE CASCADE,
    cluster_id  INTEGER      REFERENCES pet_clusters(id)  ON DELETE SET NULL,
    species     TEXT NOT NULL,       -- "dog", "cat", "bird", etc.
    confidence  REAL NOT NULL,       -- MobileNetV2 classification confidence
    embedding   BLOB,                -- raw 1000-dim f32 le-bytes from MobileNetV2 logits
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_pet_det_photo   ON pet_detections(photo_id);
CREATE INDEX IF NOT EXISTS idx_pet_det_user    ON pet_detections(user_id);
CREATE INDEX IF NOT EXISTS idx_pet_det_cluster ON pet_detections(cluster_id);
CREATE INDEX IF NOT EXISTS idx_pet_det_species ON pet_detections(user_id, species);
