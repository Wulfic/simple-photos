-- Tombstones for Takeout-derived albums the user deleted.
--
-- Album reconstruction rebuilds album manifests from `photo_source_albums` on
-- every client, keyed by a deterministic id derived from (source, album_name).
-- That made deleting a reconstructed album impossible: the delete removed the
-- local album and its manifest blob, and then the very next reconstruction pass
-- (a fresh session resets the client's "already materialized" state) recreated
-- it from the same server-side membership. The user's curation was silently
-- undone, forever, on every device.
--
-- A tombstone records the deletion authoritatively so reconstruction skips the
-- album everywhere, not just on the device that deleted it.
--
-- Keyed by (source, album_name) — the album's *identity*, matching the key
-- clients derive the album id from — rather than by the client-side album id, so
-- a tombstone written by web is understood by Android and vice versa. Deliberately
-- NOT keyed by album_title: retitling an album must not resurrect it.
--
-- No E2E regression: `photo_source_albums.album_name` is already plaintext
-- server-side (the server captured it from the folder name at import), so
-- storing the same name here reveals nothing new. Membership is untouched — the
-- photos stay in the library, exactly as "delete album, keep photos" means.
CREATE TABLE IF NOT EXISTS dismissed_source_albums (
    user_id      TEXT NOT NULL,
    source       TEXT NOT NULL DEFAULT 'google_takeout',
    album_name   TEXT NOT NULL,
    dismissed_at TEXT NOT NULL,
    PRIMARY KEY (user_id, source, album_name)
);

-- The reconstruction read path: "which albums has this user dismissed?".
CREATE INDEX IF NOT EXISTS idx_dismissed_source_albums_user
    ON dismissed_source_albums (user_id);
