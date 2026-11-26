-- Migration 041: trash keeps the photo, not just its bytes (TODO.md F11)
--
-- Both trash paths DELETE the `photos` row, and `foreign_keys` is ON, so the
-- delete CASCADEs every child row (edit copies, source-album membership, ...).
-- `trash_items` only ever held a hand-picked subset of columns, and restore
-- rebuilt the row from that subset — or, on the blob path every client uses,
-- did not rebuild it at all: a restored registered (Android) photo came back
-- as bytes no gallery lists.
--
-- The `blobs` rows had the same disease: restore hand-built them with
-- `size_bytes` from the client's trash request (the phone sends the ORIGINAL
-- file's size, not the blob's), `size_bytes = 0` for the thumbnail, and no
-- `blob_format`. `size_bytes` is the download's Content-Length, so a restored
-- blob was served truncated (the thumbnail: empty), and a v2 blob came back as
-- v1. `trash_blob_snapshots` keeps each deleted `blobs` row verbatim.
--
-- One row per `photos` row a trash call deleted (the blob path can delete more
-- than one — `encrypted_blob_id` is not unique). `photo_row` is the whole row
-- as JSON, `children` is {table: [rows]} for the user-authored child tables
-- (`trash::snapshot::SNAPSHOT_CHILDREN`). Both are written by
-- `trash::snapshot::capture` and only read by `trash::snapshot::restore`.
--
-- The FK means permanent delete, empty-trash and the retention purge drop the
-- snapshot with its trash row; nothing else has to know this table exists.
--
-- Reverse (forward-only sqlx migrations have no down file):
--   DROP TABLE IF EXISTS trash_blob_snapshots;
--   DROP TABLE IF EXISTS trash_photo_snapshots;
-- Loses only the snapshots of currently-trashed items: their restores fall
-- back to the pre-041 behavior. The binary may be rolled back without it
-- (`db.rs` migrates with `set_ignore_missing(true)`).

CREATE TABLE IF NOT EXISTS trash_photo_snapshots (
    trash_id   TEXT NOT NULL REFERENCES trash_items(id) ON DELETE CASCADE,
    photo_id   TEXT NOT NULL,
    photo_row  TEXT NOT NULL,
    children   TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    PRIMARY KEY (trash_id, photo_id)
);

CREATE TABLE IF NOT EXISTS trash_blob_snapshots (
    trash_id   TEXT NOT NULL REFERENCES trash_items(id) ON DELETE CASCADE,
    blob_id    TEXT NOT NULL,
    blob_row   TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (trash_id, blob_id)
);
