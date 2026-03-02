-- Edit copies: non-destructive "Save Copy" creates a named metadata-only version
-- of a photo/video/audio without duplicating the actual file.
CREATE TABLE IF NOT EXISTS edit_copies (
    id TEXT PRIMARY KEY,
    photo_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    edit_metadata TEXT NOT NULL,   -- JSON: same shape as crop_metadata + trimStart/trimEnd
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY (photo_id) REFERENCES photos(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_edit_copies_photo ON edit_copies(photo_id, user_id);
