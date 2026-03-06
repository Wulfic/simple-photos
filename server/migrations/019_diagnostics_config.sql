-- Diagnostics configuration: master enable/disable toggle for server diagnostics collection.
-- When disabled, expensive metrics collection (disk walks, table counts, etc.) is skipped.
-- Default is 'true' (diagnostics collection is enabled).
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('diagnostics_enabled', 'true');

-- Client diagnostics: whether web and mobile clients should collect and send diagnostic logs.
-- This allows central control — admins can enable/disable client-side logging from the dashboard.
INSERT OR IGNORE INTO server_settings (key, value) VALUES ('client_diagnostics_enabled', 'true');
