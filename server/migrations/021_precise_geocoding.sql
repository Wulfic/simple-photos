-- Opt-in precise (street-level) reverse geocoding.
--
-- The whole database is encrypted at rest via SQLCipher, so these columns are
-- stored plaintext *inside* the encrypted file — no per-column encryption
-- needed.  Existing coarse geo columns (geo_city/state/country) are unchanged.

-- Street-level fields resolved from the opt-in online geocoder.
ALTER TABLE photos ADD COLUMN geo_street       TEXT;
ALTER TABLE photos ADD COLUMN geo_house_number TEXT;
-- Pre-formatted label used directly in memory/trip titles
-- (e.g. "86 Nelson Blvd, Springfield, Ohio").
ALTER TABLE photos ADD COLUMN geo_address      TEXT;
-- Resolution state: NULL = pending, 'ok' = resolved, '' = attempted but the
-- provider had no address (mirrors the geo_city '' sentinel convention).
ALTER TABLE photos ADD COLUMN geo_precise_status TEXT;

-- Find photos awaiting precise resolution without scanning the whole table.
-- Only the coarse-resolved set is ever eligible (we need a city first).
CREATE INDEX idx_photos_geo_precise_pending
    ON photos(user_id)
    WHERE geo_city IS NOT NULL AND geo_city != '' AND geo_precise_status IS NULL;

-- Reverse-geocode dedup cache, keyed by rounded coordinate so repeated
-- locations collapse to a single network call (provider-policy compliance).
CREATE TABLE IF NOT EXISTS geo_address_cache (
    coord_key   TEXT PRIMARY KEY,   -- "lat4,lon4" rounded to ~11 m
    payload     TEXT NOT NULL,      -- JSON PreciseAddress
    source      TEXT,               -- 'nominatim' | 'photon'
    fetched_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
