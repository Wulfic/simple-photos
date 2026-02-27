-- Photo tags: lightweight tagging system for search and organization.
-- Tags are user-scoped — each user has their own tag namespace.

CREATE TABLE photo_tags (
    photo_id    TEXT NOT NULL,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tag         TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    PRIMARY KEY (photo_id, user_id, tag)
);

-- Index for searching photos by tag
CREATE INDEX idx_photo_tags_user_tag ON photo_tags(user_id, tag);

-- Index for listing tags on a specific photo
CREATE INDEX idx_photo_tags_photo ON photo_tags(photo_id, user_id);

-- Index for listing all unique tags for a user
CREATE INDEX idx_photo_tags_user ON photo_tags(user_id);
