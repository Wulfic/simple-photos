-- Backup mode: when set to 'backup', this server acts as a backup server
-- and broadcasts its presence on the local network for auto-discovery.
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('backup_mode', 'primary');

-- Track the last automatic scan time so we can trigger every 24 hours.
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('last_auto_scan', '');
