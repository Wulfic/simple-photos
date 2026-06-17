"""
E2E: Smart location albums through the **real** reverse-geocoder.

This test fills the seam that `test_53` and `test_71` deliberately leave
uncovered:

  • `test_53_geo_pipeline`  — uploads GPS JPEGs but explicitly does NOT
    reverse-geocode (lat/lon → city).  It only checks coordinate storage,
    scrubbing, map and timeline.
  • `test_71_smart_trip_albums` — tests trip clustering by writing
    `geo_city` straight into SQLite, bypassing the geocoder entirely.

Neither exercises the full chain that a real user hits:

    upload GPS-tagged JPEG
        → EXIF GPS extracted into latitude/longitude   (upload.rs)
        → background processor resolves geo_city from cities500.txt
                                                        (geo/processor.rs)
        → /api/geo/trips and /api/geo/memories return albums
                                                        (geo/handlers.rs)

That middle hop — the offline reverse-geocoder + the backfill loop — is
exactly where "location albums never show up" lives.  This test drives it
honestly: it enables geo, uploads real-coordinate photos with real dates,
waits for the 2 s backfill cycle to resolve cities from the bundled
GeoNames dataset, then asserts trips and memories actually materialise.

Requires the GeoNames `cities500.txt` (conftest points `dataset_path` at
the copy in `server/data/`).  If that file is absent the geo backfill can
never resolve a city, so the test is skipped rather than failing red.
"""

from __future__ import annotations

import os
import time

import pytest

from helpers import (
    APIClient,
    generate_test_jpeg_with_gps,
    random_username,
    unique_filename,
)
from conftest import USER_PASSWORD, SERVER_DIR


# Skip the whole module when the dataset the geocoder needs is not present;
# without it geo_city is never resolved and these assertions are meaningless.
_DATASET = os.path.join(SERVER_DIR, "data", "cities500.txt")
pytestmark = pytest.mark.skipif(
    not os.path.isfile(_DATASET),
    reason="GeoNames cities500.txt not present — reverse geocoding unavailable",
)


# ── Real-world coordinates that resolve cleanly in cities500 ─────────────
# All photos for a city share IDENTICAL coordinates so they all resolve to
# the same geo_city (a few-metre jitter could otherwise split across
# neighbouring cities and break the count assertions).

NYC_LAT, NYC_LON = 40.7484, -73.9857     # Empire State Building → US
TOKYO_LAT, TOKYO_LON = 35.6595, 139.7006  # Shibuya            → JP


def _wait_for_cities(client: APIClient, expected: int, timeout: float = 45.0) -> list:
    """Poll the locations endpoint until the background backfill has resolved
    at least `expected` distinct cities, or `timeout` elapses.

    The test server runs `poll_interval_secs = 2`, so resolution normally
    lands within a few seconds; the generous timeout absorbs CI jitter and
    the first-cycle dataset load (~25 MB parse)."""
    deadline = time.time() + timeout
    locations: list = []
    while time.time() < deadline:
        locations = client.geo_locations()
        if len(locations) >= expected:
            return locations
        time.sleep(1.0)
    return locations


@pytest.fixture(scope="module")
def geo_albums_user(primary_admin, primary_server):
    """Create a user, enable geo, upload geotagged photos across two cities
    and two dates, then wait for the real geocoder to resolve them."""
    username = random_username("geoalb_")
    primary_admin.admin_create_user(username, USER_PASSWORD)

    client = APIClient(primary_server.base_url)
    client.login(username, USER_PASSWORD)

    # Opt this user into geocoding (config default is off).
    r = client.geo_update_settings(enabled=True)
    assert r.status_code == 200, r.text

    # ── NYC: 6 photos across two consecutive days (3 + 3) ───────────────
    # → one multi-day trip (6 photos, 2 days) AND two same-day memories.
    nyc_plan = [
        ("2025:05:10 09:00:00", "2025-05-10"),
        ("2025:05:10 12:00:00", "2025-05-10"),
        ("2025:05:10 18:00:00", "2025-05-10"),
        ("2025:05:11 10:00:00", "2025-05-11"),
        ("2025:05:11 14:00:00", "2025-05-11"),
        ("2025:05:11 20:00:00", "2025-05-11"),
    ]
    # ── Tokyo: 5 photos, single day burst ──────────────────────────────
    # → one single-day trip (5 photos) AND one same-day memory.
    tokyo_plan = [
        (f"2025:09:04 0{h}:00:00", "2025-09-04") for h in range(1, 6)
    ]

    nyc_ids: list[str] = []
    for i, (date_str, _day) in enumerate(nyc_plan):
        # Vary width so each JPEG hashes uniquely (content-hash dedup would
        # otherwise collapse byte-identical uploads into a single row).
        content = generate_test_jpeg_with_gps(
            NYC_LAT, NYC_LON, date_str=date_str, width=4 + i, height=4 + i
        )
        res = client.upload_photo(filename=unique_filename(".jpg"), content=content)
        nyc_ids.append(res["photo_id"])

    tokyo_ids: list[str] = []
    for i, (date_str, _day) in enumerate(tokyo_plan):
        content = generate_test_jpeg_with_gps(
            TOKYO_LAT, TOKYO_LON, date_str=date_str, width=20 + i, height=20 + i
        )
        res = client.upload_photo(filename=unique_filename(".jpg"), content=content)
        tokyo_ids.append(res["photo_id"])

    # Wait for the real backfill to resolve both cities.
    locations = _wait_for_cities(client, expected=2)
    assert len(locations) >= 2, (
        f"Geocoder never resolved 2 cities from cities500.txt; got {locations}. "
        "If this is empty the backfill loop / dataset path is broken — the exact "
        "failure that hides location albums in real use."
    )

    return {
        "client": client,
        "nyc_ids": nyc_ids,
        "tokyo_ids": tokyo_ids,
        "locations": locations,
    }


# ── Assertions ──────────────────────────────────────────────────────────


def test_locations_resolved_by_real_geocoder(geo_albums_user):
    """The offline geocoder must turn lat/lon into real city rows."""
    locations = geo_albums_user["locations"]
    codes = {loc["country_code"] for loc in locations}
    assert "US" in codes, f"NYC photos did not resolve to a US city: {locations}"
    assert "JP" in codes, f"Tokyo photos did not resolve to a JP city: {locations}"

    # Every resolved location must carry a non-empty city name (not the ''
    # 'attempted but unresolved' sentinel).
    for loc in locations:
        assert loc["city"], f"Empty city name in resolved location: {loc}"
        assert loc["photo_count"] >= 1


def test_trips_materialise_from_real_pipeline(geo_albums_user):
    """`/api/geo/trips` must surface both clusters end-to-end."""
    client: APIClient = geo_albums_user["client"]
    trips = client.get("/api/geo/trips").json()

    us_trips = [t for t in trips if t["country_code"] == "US"]
    jp_trips = [t for t in trips if t["country_code"] == "JP"]

    assert len(us_trips) == 1, f"Expected one US trip, got {us_trips}"
    assert us_trips[0]["photo_count"] == 6
    assert us_trips[0]["day_count"] == 2
    assert us_trips[0]["start_date"] == "2025-05-10"
    assert us_trips[0]["end_date"] == "2025-05-11"

    assert len(jp_trips) == 1, f"Expected one JP trip, got {jp_trips}"
    assert jp_trips[0]["photo_count"] == 5
    assert jp_trips[0]["day_count"] == 1


def test_memories_materialise_from_real_pipeline(geo_albums_user):
    """`/api/geo/memories` clusters same-city/same-day groups (cnt >= 3)."""
    client: APIClient = geo_albums_user["client"]
    memories = client.get("/api/geo/memories").json()

    # NYC: two days × 3 photos → two memories.  Tokyo: one day × 5 → one.
    counts = sorted(m["photo_count"] for m in memories)
    assert counts == [3, 3, 5], f"Unexpected memory clusters: {memories}"


def test_trip_photos_endpoint_real(geo_albums_user):
    """Drilling into a trip returns exactly its member photos."""
    client: APIClient = geo_albums_user["client"]
    trips = client.get("/api/geo/trips").json()
    us_trip = next(t for t in trips if t["country_code"] == "US")

    photos = client.get(f"/api/geo/trips/{us_trip['id']}/photos").json()
    returned = {p["id"] for p in photos}
    assert returned == set(geo_albums_user["nyc_ids"]), (
        "Trip photos endpoint did not return exactly the NYC uploads"
    )
