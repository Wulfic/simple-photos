-- Audio backup setting: whether audio files are included in backup sync.
-- Default is 'false' (audio files are NOT backed up by default).
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('audio_backup_enabled', 'false');
