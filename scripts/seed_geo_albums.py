#!/usr/bin/env python3
"""
Seed a running Simple-Photos server with geotagged photos so the smart
**location albums** (Trips + Memories) actually appear in the web UI.

Why this exists
---------------
Location albums require a chain of preconditions that are easy to miss in
manual testing, which is why they "never show up":

  1. Geolocation must be ENABLED for the user (off by default — privacy).
  2. Photos must carry GPS EXIF  → server fills latitude/longitude.
  3. Photos must carry a date    → server fills taken_at (trips/memories
     filter `taken_at IS NOT NULL`).
  4. Enough clustered photos:
       • Trip   = >= 5 photos, same city, consecutive dates <= 3 days apart
       • Memory = >= 3 photos, same city, same calendar day
  5. The GeoNames cities500.txt dataset must be installed server-side.
  6. The background geo-processor poll cycle must run (every 5 min in
     production; 2 s in the test harness).

This script satisfies 1–4 and 6, then waits for resolution and prints the
trips/memories the server produced.  (5 is a server install concern — if
no cities resolve, your dataset is missing; see geo/processor.rs logs.)

Usage
-----
    python scripts/seed_geo_albums.py \
        --url http://127.0.0.1:3000 \
        --username alice --password 'S3cret!'

    # create the user first (requires it to be the first/admin or an
    # existing admin token flow — otherwise log in as an existing user):
    python scripts/seed_geo_albums.py --url ... --username alice \
        --password 'S3cret!' --register

Nothing here is destructive: it only uploads new photos and flips the
caller's own `geo_enabled` setting on.
"""

from __future__ import annotations

import argparse
import os
import sys
import time

# Reuse the EXIF GPS JPEG generator from the e2e helpers so the bytes the
# server parses are identical to what the test-suite validates.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "tests"))
from helpers import APIClient, generate_test_jpeg_with_gps, unique_filename  # noqa: E402


# ── Curated itinerary: real coordinates + dates that trigger albums ──────
# Each entry: (label, lat, lon, [ (exif_date, ...) ]).  Coordinates are
# identical within a city so every photo resolves to the same geo_city.

ITINERARY = [
    # London — long weekend (3 days, 7 photos) → multi-day trip + memories
    ("London", 51.5007, -0.1246, [
        "2025:06:06 09:00:00", "2025:06:06 13:00:00", "2025:06:06 19:00:00",
        "2025:06:07 10:00:00", "2025:06:07 16:00:00",
        "2025:06:08 11:00:00", "2025:06:08 15:00:00",
    ]),
    # Tokyo — single-day burst (6 photos) → single-day trip + one memory
    ("Tokyo", 35.6595, 139.7006, [
        "2025:09:04 08:00:00", "2025:09:04 09:30:00", "2025:09:04 11:00:00",
        "2025:09:04 13:00:00", "2025:09:04 17:00:00", "2025:09:04 20:00:00",
    ]),
    # New York — two-day city break (5 photos) → trip + memories
    ("New York", 40.7484, -73.9857, [
        "2025:11:21 10:00:00", "2025:11:21 14:00:00", "2025:11:21 21:00:00",
        "2025:11:22 09:00:00", "2025:11:22 18:00:00",
    ]),
]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--url", default=os.environ.get("SP_URL", "http://127.0.0.1:3000"),
                    help="Base server URL (default: %(default)s)")
    ap.add_argument("--username", required=True)
    ap.add_argument("--password", required=True)
    ap.add_argument("--register", action="store_true",
                    help="Register the user before logging in (first-run/open signup only)")
    ap.add_argument("--wait", type=float, default=320.0,
                    help="Seconds to wait for city resolution (prod poll = 5 min; default %(default)s)")
    args = ap.parse_args()

    client = APIClient(args.url)

    if args.register:
        try:
            client.register(args.username, args.password)
            print(f"[seed] registered user {args.username!r}")
        except Exception as e:  # noqa: BLE001 — best-effort; fall through to login
            print(f"[seed] register skipped/failed ({e}); trying login")

    client.login(args.username, args.password)
    print(f"[seed] logged in as {args.username!r} @ {args.url}")

    # 1) Enable geolocation for this user (off by default).
    r = client.geo_update_settings(enabled=True)
    if r.status_code != 200:
        print(f"[seed] ERROR enabling geo: HTTP {r.status_code} {r.text}")
        return 1
    print("[seed] geolocation enabled for user")

    # 2) Upload the geotagged itinerary.
    total = 0
    for city, lat, lon, dates in ITINERARY:
        for i, date_str in enumerate(dates):
            # Vary dimensions so each JPEG hashes uniquely (content-hash dedup
            # would otherwise collapse byte-identical uploads into one row).
            content = generate_test_jpeg_with_gps(
                lat, lon, date_str=date_str, width=8 + i, height=8 + i
            )
            client.upload_photo(filename=unique_filename(".jpg"), content=content)
            total += 1
        print(f"[seed] uploaded {len(dates)} geotagged photos for {city}")
    print(f"[seed] {total} photos uploaded total")

    # 3) Wait for the background geo-processor to resolve cities.
    print(f"[seed] waiting up to {args.wait:.0f}s for reverse-geocoding "
          f"(server poll cycle)...")
    deadline = time.time() + args.wait
    locations: list = []
    while time.time() < deadline:
        try:
            locations = client.geo_locations()
        except Exception:  # noqa: BLE001
            locations = []
        if len(locations) >= len(ITINERARY):
            break
        time.sleep(3.0)

    if not locations:
        print("[seed] WARNING: no cities resolved. Likely causes: GeoNames "
              "cities500.txt missing server-side, or geo disabled in config. "
              "Check the server log for 'cities500.txt not found'.")
        return 2

    print(f"[seed] resolved {len(locations)} location(s):")
    for loc in locations:
        print(f"        • {loc['city']}, {loc.get('country','?')} "
              f"({loc['country_code']}) — {loc['photo_count']} photos")

    # 4) Report the smart albums the server now exposes.
    trips = client.get("/api/geo/trips").json()
    memories = client.get("/api/geo/memories").json()
    print(f"\n[seed] /api/geo/trips → {len(trips)} trip album(s):")
    for t in trips:
        print(f"        • {t['name']}  ({t['photo_count']} photos, "
              f"{t['day_count']} day(s))")
    print(f"[seed] /api/geo/memories → {len(memories)} memory album(s):")
    for m in memories:
        print(f"        • {m['name']}  ({m['photo_count']} photos)")

    print("\n[seed] Done. Open the web app → Albums; the Trips and Memories "
          "sections should now be populated.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
