-- Remove conversion-related server_settings entries.
-- These were used by the background conversion pipeline which has been removed.
-- All supported formats are now browser-native — no conversion required.

DELETE FROM server_settings WHERE key LIKE 'blob_converted_%';
DELETE FROM server_settings WHERE key LIKE 'conv_failed_%';
