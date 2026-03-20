-- Force encrypted mode: the server no longer supports plain (unencrypted) storage.
-- All photos must be encrypted with client-side AES-256-GCM.

-- 1. Set encryption_mode to 'encrypted' unconditionally.
--    Existing installs that were in 'plain' mode will be auto-migrated on startup.
UPDATE server_settings SET value = 'encrypted' WHERE key = 'encryption_mode';

-- 2. Drop the encryption_migration table — mode toggling is no longer supported.
--    The auto-migration on startup uses its own tracking mechanism.
DROP TABLE IF EXISTS encryption_migration;
