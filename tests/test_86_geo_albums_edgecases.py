"""E2E: every smart-location-album EDGE CASE, end-to-end through the real geocoder.

Where ``test_85`` proves the happy path (a couple of cities resolve into trips
and memories), this drives the full edge-case matrix defined once in
``tests/geo_fixtures.py`` and shared with ``scripts/generate_geo_samples.py``
(the captioned desktop samples) so the samples you eyeball and the assertions
here can never drift:

  * multi-day trip + per-day memories            (paris)
  * single-day burst still counts as a trip      (tokyo)
  * exactly 3 photos/day = memory, < 5 = no trip (rome)
  * a gap of exactly MAX_GAP_DAYS stays one trip (barcelona)
  * same city visited twice (>3-day gap) = 2 trips (amsterdam)
  * below the memory threshold (2 < 3)           (lisbon)
  * GPS but no EXIF date -> stamped to import day (sydney)
  * a date but no GPS -> timeline only           (timeline_nogps)
  * unresolvable coordinates -> '' sentinel, no album, no wedge (ocean)
  * precise (street-level) reverse geocoding     (nyc; precise path is covered
    separately/offline — here it just contributes a normal memory)

Skipped when ``cities500.txt`` is absent (identical guard to test_85), since
without the dataset no city resolves and every assertion is meaningless.
"""

from __future__ import annotations

import os
import time

import pytest

from helpers import APIClient, random_username, unique_filename
from conftest import USER_PASSWORD, SERVER_DIR
from geo_fixtures import (
    GROUPS,
    EXPLICIT_DAYS,
    EXPECT_IMPORT_DATED_MEMORY_COUNT,
    EXPECT_IMPORT_DATED_TRIP_COUNT,
    EXPECT_LOCATION_CITY_COUNT,
    EXPECT_MEMORY_PHOTO_COUNTS_DATED,
    EXPECT_TRIP_PHOTO_COUNTS_DATED,
    memory_day,
    photo_bytes,
    total_photos,
)

_DATASET = os.path.join(SERVER_DIR, "data", "cities500.txt")
pytestmark = pytest.mark.skipif(
    not os.path.isfile(_DATASET),
    reason="GeoNames cities500.txt not present — reverse geocoding unavailable",
)

# Derived from the fixtures so there are no magic numbers to drift.
_GPS_PHOTOS = sum(g.count for g in GROUPS if g.has_gps)        # incl. ocean sentinel
_NOGPS_PHOTOS = sum(g.count for g in GROUPS if not g.has_gps)  # timeline_nogps


@pytest.fixture(scope="module")
def edgecase_user(primary_admin, primary_server):
    """A fresh user with geo enabled, loaded with the full edge-case set, after
    the real background geocoder has had a chance to resolve it.

    All geo queries are per-user (``WHERE user_id = ?``), so this user's counts
    are isolated from anything else running in the same session.

    NOTE: the no-date group is stamped with the server's current time; this
    test assumes the wall clock is later than the explicit 2025 sample dates
    (true for any real run) so its album never lands inside EXPLICIT_DAYS.
    """
    username = random_username("geoedge_")
    primary_admin.admin_create_user(username, USER_PASSWORD)

    client = APIClient(primary_server.base_url)
    client.login(username, USER_PASSWORD)
    assert client.geo_update_settings(enabled=True).status_code == 200

    seq = 0
    for g in GROUPS:
        for idx in range(g.count):
            client.upload_photo(
                filename=unique_filename(".jpg"),
                content=photo_bytes(g, idx, seq, captioned=False),
            )
            seq += 1

    # Wait for the backfill loop (poll_interval_secs = 2 in the test harness)
    # to resolve the expected number of cities.
    deadline = time.time() + 60
    locations: list = []
    while time.time() < deadline:
        locations = client.geo_locations()
        if len(locations) >= EXPECT_LOCATION_CITY_COUNT:
            break
        time.sleep(1.0)

    return {"client": client, "locations": locations, "uploaded": seq}


def test_all_photos_uploaded(edgecase_user):
    assert edgecase_user["uploaded"] == total_photos()


def test_locations_resolved(edgecase_user):
    """Every group with resolvable GPS coordinates becomes exactly one location;
    the ocean coordinate (no nearby city) does NOT."""
    assert len(edgecase_user["locations"]) == EXPECT_LOCATION_CITY_COUNT, (
        f"got {edgecase_user['locations']}"
    )


def test_dated_trips(edgecase_user):
    """Trips from the explicitly-dated groups: multi-day, single-day burst, the
    exact 3-day gap boundary, and a city visited twice (two trips)."""
    trips = edgecase_user["client"].get("/api/geo/trips").json()
    dated = sorted(t["photo_count"] for t in trips if t["start_date"] in EXPLICIT_DAYS)
    assert dated == EXPECT_TRIP_PHOTO_COUNTS_DATED, trips


def test_dated_memories(edgecase_user):
    """Memories cluster same-city/same-day groups with cnt >= 3; 2-photo days
    and the below-threshold group produce none."""
    mems = edgecase_user["client"].get("/api/geo/memories").json()
    dated = sorted(m["photo_count"] for m in mems if memory_day(m) in EXPLICIT_DAYS)
    assert dated == EXPECT_MEMORY_PHOTO_COUNTS_DATED, mems


def test_no_date_group_is_stamped_to_import_time(edgecase_user):
    """A geotagged photo with no EXIF date is never stored NULL — the server
    falls back to the import time (upload.rs), so the no-date group forms one
    extra trip + memory on a day outside the explicit sample dates."""
    client = edgecase_user["client"]
    trips = client.get("/api/geo/trips").json()
    mems = client.get("/api/geo/memories").json()
    import_trips = [t["photo_count"] for t in trips if t["start_date"] not in EXPLICIT_DAYS]
    import_mems = [m["photo_count"] for m in mems if memory_day(m) not in EXPLICIT_DAYS]
    assert import_trips == [EXPECT_IMPORT_DATED_TRIP_COUNT], import_trips
    assert import_mems == [EXPECT_IMPORT_DATED_MEMORY_COUNT], import_mems


def test_sentinel_and_no_gps_counts(edgecase_user):
    """The ocean coordinate is stored (counts toward photos_with_location) but
    resolves to the '' sentinel (excluded from unique_cities), and the no-GPS
    group counts as photos_without_location."""
    s = edgecase_user["client"].geo_settings()
    assert s["photos_with_location"] == _GPS_PHOTOS, s
    assert s["photos_without_location"] == _NOGPS_PHOTOS, s
    assert s["unique_cities"] == EXPECT_LOCATION_CITY_COUNT, s


def test_trip_photos_endpoint_returns_members(edgecase_user):
    """Drilling into a dated trip returns exactly its member photos (count
    matches the album), proving the trip-id round-trip parses correctly."""
    client = edgecase_user["client"]
    trips = client.get("/api/geo/trips").json()
    dated = [t for t in trips if t["start_date"] in EXPLICIT_DAYS]
    assert dated, "no dated trips to drill into"
    for t in dated:
        photos = client.get(f"/api/geo/trips/{t['id']}/photos").json()
        assert len(photos) == t["photo_count"], (t["id"], len(photos), t["photo_count"])
