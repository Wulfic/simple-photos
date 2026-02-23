CREATE TABLE users (
    id                    TEXT PRIMARY KEY,
    username              TEXT NOT NULL UNIQUE,
    password_hash         TEXT NOT NULL,
    created_at            TEXT NOT NULL,
    storage_quota_bytes   INTEGER NOT NULL DEFAULT 10737418240,
    totp_secret           TEXT,
    totp_enabled          INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE totp_backup_codes (
    id        TEXT PRIMARY KEY,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    used      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_totp_backup_user ON totp_backup_codes(user_id);

CREATE TABLE refresh_tokens (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL UNIQUE,
    expires_at  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    revoked     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_refresh_tokens_user ON refresh_tokens(user_id);

CREATE TABLE blobs (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blob_type    TEXT NOT NULL,
    size_bytes   INTEGER NOT NULL,
    client_hash  TEXT,
    upload_time  TEXT NOT NULL,
    storage_path TEXT NOT NULL
);
CREATE INDEX idx_blobs_user_type_time ON blobs(user_id, blob_type, upload_time);
CREATE INDEX idx_blobs_client_hash ON blobs(user_id, client_hash) WHERE client_hash IS NOT NULL;
