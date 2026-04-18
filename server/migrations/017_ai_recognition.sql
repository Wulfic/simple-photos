-- Migration 017: AI Face & Object Recognition
--
-- Adds tables for face detections, face clusters, object detections,
-- and per-user settings (also used by later geolocation module).

-- Per-user settings table (shared across modules).
CREATE TABLE IF NOT EXISTS user_settings (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, key)
);

-- Face clusters (one per recognized person identity).
-- Created BEFORE face_detections so the FK reference is valid.
CREATE TABLE IF NOT EXISTS face_clusters (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label           TEXT,
    representative  TEXT,
    photo_count     INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_face_cluster_user ON face_clusters(user_id);
CREATE INDEX IF NOT EXISTS idx_face_cluster_label ON face_clusters(user_id, label);

-- Face detections (raw per-photo-per-face records).
CREATE TABLE IF NOT EXISTS face_detections (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id    TEXT NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cluster_id  INTEGER REFERENCES face_clusters(id) ON DELETE SET NULL,
    bbox_x      REAL NOT NULL,
    bbox_y      REAL NOT NULL,
    bbox_w      REAL NOT NULL,
    bbox_h      REAL NOT NULL,
    confidence  REAL NOT NULL,
    embedding   BLOB,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_face_det_photo ON face_detections(photo_id);
CREATE INDEX IF NOT EXISTS idx_face_det_user ON face_detections(user_id);
CREATE INDEX IF NOT EXISTS idx_face_det_cluster ON face_detections(cluster_id);

-- Object detections (per-photo).
CREATE TABLE IF NOT EXISTS object_detections (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id    TEXT NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    class_name  TEXT NOT NULL,
    confidence  REAL NOT NULL,
    bbox_x      REAL NOT NULL,
    bbox_y      REAL NOT NULL,
    bbox_w      REAL NOT NULL,
    bbox_h      REAL NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_obj_det_photo ON object_detections(photo_id);
CREATE INDEX IF NOT EXISTS idx_obj_det_user ON object_detections(user_id);
CREATE INDEX IF NOT EXISTS idx_obj_det_class ON object_detections(user_id, class_name);

-- Track which photos have been processed by the AI engine.
-- Avoids re-processing on restart without complex JOIN queries.
CREATE TABLE IF NOT EXISTS ai_processed_photos (
    photo_id   TEXT NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    processed_at TEXT NOT NULL,
    PRIMARY KEY (photo_id, user_id)
);
